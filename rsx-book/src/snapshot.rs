use crate::book::BookState;
use crate::book::Orderbook;
use crate::compression::CompressionMap;
use crate::level::PriceLevel;
use crate::order::OrderSlot;
use crate::slab::Slab;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::SymbolConfig;
use std::io;
use std::io::Read;
use std::io::Write;

const SNAPSHOT_MAGIC: u32 = 0x5258_534E; // "RXSN"
const SNAPSHOT_VERSION: u32 = 1;

/// Save orderbook snapshot to writer.
/// Returns Err if book is migrating.
pub fn save(book: &Orderbook, w: &mut dyn Write) -> io::Result<()> {
    if book.state == BookState::Migrating {
        return Err(io::Error::other("cannot snapshot during migration"));
    }

    // Header
    w.write_all(&SNAPSHOT_MAGIC.to_le_bytes())?;
    w.write_all(&SNAPSHOT_VERSION.to_le_bytes())?;
    w.write_all(&book.sequence.to_le_bytes())?;

    // Config
    w.write_all(&book.config.symbol_id.to_le_bytes())?;
    w.write_all(&[book.config.price_decimals])?;
    w.write_all(&[book.config.qty_decimals])?;
    w.write_all(&book.config.tick_size.to_le_bytes())?;
    w.write_all(&book.config.lot_size.to_le_bytes())?;

    // Compression map
    w.write_all(&book.compression.mid_price.to_le_bytes())?;

    // BBA
    w.write_all(&book.best_bid_tick.to_le_bytes())?;
    w.write_all(&book.best_ask_tick.to_le_bytes())?;

    // Slab metadata
    let capacity = book.orders.capacity();
    let bump = book.orders.len();
    w.write_all(&capacity.to_le_bytes())?;
    w.write_all(&bump.to_le_bytes())?;

    // Active orders: count then entries
    let mut active_count: u32 = 0;
    for i in 0..bump {
        if book.orders.get(i).is_active() {
            active_count += 1;
        }
    }
    w.write_all(&active_count.to_le_bytes())?;

    for i in 0..bump {
        let slot = book.orders.get(i);
        if slot.is_active() {
            w.write_all(&i.to_le_bytes())?;
            write_order(w, slot)?;
        }
    }

    // Non-empty levels: count then entries
    let total_levels = book.active_levels.len() as u32;
    w.write_all(&total_levels.to_le_bytes())?;
    let mut level_count: u32 = 0;
    for lvl in &book.active_levels {
        if lvl.order_count > 0 {
            level_count += 1;
        }
    }
    w.write_all(&level_count.to_le_bytes())?;

    for (idx, lvl) in book.active_levels.iter().enumerate() {
        if lvl.order_count > 0 {
            w.write_all(&(idx as u32).to_le_bytes())?;
            write_level(w, lvl)?;
        }
    }

    // User state
    book.users.write_snapshot(w)?;

    Ok(())
}

/// Load orderbook from snapshot. Returns Box
/// to avoid stack overflow from event_buf.
pub fn load(r: &mut dyn Read) -> io::Result<Box<Orderbook>> {
    // Header
    let magic = read_u32(r)?;
    if magic != SNAPSHOT_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid snapshot magic",
        ));
    }
    let version = read_u32(r)?;
    if version != SNAPSHOT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported snapshot version {}", version,),
        ));
    }
    let sequence = read_u64(r)?;

    // Config
    let symbol_id = read_u32(r)?;
    let price_decimals = read_u8(r)?;
    let qty_decimals = read_u8(r)?;
    let tick_size = read_i64(r)?;
    let lot_size = read_i64(r)?;
    let config = SymbolConfig {
        symbol_id,
        price_decimals,
        qty_decimals,
        tick_size,
        lot_size,
    };

    // Compression
    let mid_price = read_i64(r)?;
    let compression = CompressionMap::new(mid_price, tick_size);

    // BBA
    let best_bid_tick = read_u32(r)?;
    let best_ask_tick = read_u32(r)?;

    // Slab
    let capacity = read_u32(r)?;
    let bump = read_u32(r)?;
    let active_count = read_u32(r)?;

    if bump > capacity {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("slab bump {} exceeds capacity {}", bump, capacity,),
        ));
    }
    let mut slab: Slab<OrderSlot> = Slab::new(capacity);
    // We need to set bump_next to match the
    // original. We'll rebuild the free list.
    slab.set_bump_next(bump);

    for _ in 0..active_count {
        let idx = read_u32(r)?;
        if idx >= capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "order index out of bounds",
            ));
        }
        let slot = read_order(r)?;
        *slab.get_mut(idx) = slot;
    }

    // Rebuild slab free list: any slot < bump
    // that isn't active goes on free list
    for i in (0..bump).rev() {
        if !slab.get(i).is_active() {
            slab.free(i);
        }
    }

    // Levels
    let total_levels = read_u32(r)?;
    let total = compression.total_slots() as usize;
    if total_levels as usize != total {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "level count mismatch",
        ));
    }
    let mut active_levels = vec![PriceLevel::default(); total];
    let level_count = read_u32(r)?;
    for _ in 0..level_count {
        let idx = read_u32(r)?;
        if idx as usize >= total {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "level index out of bounds",
            ));
        }
        let lvl = read_level(r)?;
        active_levels[idx as usize] = lvl;
    }

    // User state
    let users = crate::user::UserRegistry::read_snapshot(r)?;

    // Box::new to heap-allocate immediately,
    // avoiding stack overflow from event_buf.
    let mut book = Box::new(Orderbook::new(config, capacity, mid_price));
    book.active_levels = active_levels;
    book.orders = slab;
    // Level array replaced wholesale — rebuild occupancy bitmaps from it
    // (price_asc is already correct: `new` built it for this mid_price).
    book.rebuild_occupancy();
    book.best_bid_tick = best_bid_tick;
    book.best_ask_tick = best_ask_tick;
    // Derive raw BBA prices from restored levels (sawtooth index is not
    // a price proxy; snapshot format stores ticks only). Use the per-side
    // top helpers — a compressed best level may hold both sides, so the
    // FIFO head is not necessarily the best price of the tracked side.
    book.best_bid_px = book.bid_top_at(best_bid_tick).0;
    book.best_ask_px = book.ask_top_at(best_ask_tick).0;
    book.sequence = sequence;
    book.users = users;
    Ok(book)
}

fn write_order(w: &mut dyn Write, s: &OrderSlot) -> io::Result<()> {
    w.write_all(&s.price.0.to_le_bytes())?;
    w.write_all(&s.remaining_qty.0.to_le_bytes())?;
    w.write_all(&[s.side])?;
    w.write_all(&[s.flags])?;
    w.write_all(&[s.tif])?;
    w.write_all(&s.next.to_le_bytes())?;
    w.write_all(&s.prev.to_le_bytes())?;
    w.write_all(&s.tick_index.to_le_bytes())?;
    w.write_all(&s.user_id.to_le_bytes())?;
    w.write_all(&s.sequence.to_le_bytes())?;
    w.write_all(&s.original_qty.0.to_le_bytes())?;
    w.write_all(&s.timestamp_ns.to_le_bytes())?;
    w.write_all(&s.order_id_hi.to_le_bytes())?;
    w.write_all(&s.order_id_lo.to_le_bytes())?;
    Ok(())
}

fn read_order(r: &mut dyn Read) -> io::Result<OrderSlot> {
    let price = Price(read_i64(r)?);
    let remaining_qty = Qty(read_i64(r)?);
    let side = read_u8(r)?;
    let flags = read_u8(r)?;
    let tif = read_u8(r)?;
    let next = read_u32(r)?;
    let prev = read_u32(r)?;
    let tick_index = read_u32(r)?;
    let user_id = read_u32(r)?;
    let sequence = read_u32(r)?;
    let original_qty = Qty(read_i64(r)?);
    let timestamp_ns = read_u64(r)?;
    let order_id_hi = read_u64(r)?;
    let order_id_lo = read_u64(r)?;
    Ok(OrderSlot {
        price,
        remaining_qty,
        side,
        flags,
        tif,
        _pad1: [0; 5],
        next,
        prev,
        tick_index,
        _pad2: 0,
        user_id,
        sequence,
        original_qty,
        timestamp_ns,
        order_id_hi,
        order_id_lo,
        _pad4: [0; 24],
    })
}

fn write_level(w: &mut dyn Write, lvl: &PriceLevel) -> io::Result<()> {
    w.write_all(&lvl.head.to_le_bytes())?;
    w.write_all(&lvl.tail.to_le_bytes())?;
    w.write_all(&lvl.total_qty.to_le_bytes())?;
    w.write_all(&lvl.order_count.to_le_bytes())?;
    Ok(())
}

fn read_level(r: &mut dyn Read) -> io::Result<PriceLevel> {
    // bid_count/ask_count are not serialized (derivable): `load` calls
    // `rebuild_occupancy`, which recomputes them from the linked orders.
    Ok(PriceLevel {
        head: read_u32(r)?,
        tail: read_u32(r)?,
        total_qty: read_i64(r)?,
        order_count: read_u32(r)?,
        bid_count: 0,
        ask_count: 0,
    })
}

fn read_u8(r: &mut dyn Read) -> io::Result<u8> {
    let mut buf = [0u8; 1];
    r.read_exact(&mut buf)?;
    Ok(buf[0])
}

fn read_u32(r: &mut dyn Read) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64(r: &mut dyn Read) -> io::Result<u64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

fn read_i64(r: &mut dyn Read) -> io::Result<i64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(i64::from_le_bytes(buf))
}
