use crate::book::BookState;
use crate::book::Orderbook;
use crate::compression::CompressionMap;
use crate::level::PriceLevel;
use crate::order::OrderSlot;
use crate::slab::Slab;
use crate::user::UserState;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::SymbolConfig;
use rustc_hash::FxHashMap;
use std::io;
use std::io::Read;
use std::io::Write;

const SNAPSHOT_MAGIC: u32 = 0x5258_534E; // "RXSN"
const SNAPSHOT_VERSION: u32 = 1;

/// Save orderbook snapshot to writer.
/// Returns Err if book is migrating.
pub fn save(
    book: &Orderbook,
    w: &mut dyn Write,
) -> io::Result<()> {
    if book.state == BookState::Migrating {
        return Err(io::Error::other(
            "cannot snapshot during migration",
        ));
    }

    // Header
    w.write_all(&SNAPSHOT_MAGIC.to_le_bytes())?;
    w.write_all(&SNAPSHOT_VERSION.to_le_bytes())?;
    w.write_all(&book.sequence.to_le_bytes())?;

    // Config
    w.write_all(
        &book.config.symbol_id.to_le_bytes(),
    )?;
    w.write_all(
        &[book.config.price_decimals],
    )?;
    w.write_all(
        &[book.config.qty_decimals],
    )?;
    w.write_all(
        &book.config.tick_size.to_le_bytes(),
    )?;
    w.write_all(
        &book.config.lot_size.to_le_bytes(),
    )?;

    // Compression map
    w.write_all(
        &book.compression.mid_price.to_le_bytes(),
    )?;

    // BBA
    w.write_all(
        &book.best_bid_tick.to_le_bytes(),
    )?;
    w.write_all(
        &book.best_ask_tick.to_le_bytes(),
    )?;

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
    let total_levels =
        book.active_levels.len() as u32;
    w.write_all(&total_levels.to_le_bytes())?;
    let mut level_count: u32 = 0;
    for lvl in &book.active_levels {
        if lvl.order_count > 0 {
            level_count += 1;
        }
    }
    w.write_all(&level_count.to_le_bytes())?;

    for (idx, lvl) in
        book.active_levels.iter().enumerate()
    {
        if lvl.order_count > 0 {
            w.write_all(
                &(idx as u32).to_le_bytes(),
            )?;
            write_level(w, lvl)?;
        }
    }

    // User state
    w.write_all(&book.user_bump.to_le_bytes())?;
    let user_count = book.user_map.len() as u32;
    w.write_all(&user_count.to_le_bytes())?;
    for (&uid, &idx) in &book.user_map {
        w.write_all(&uid.to_le_bytes())?;
        w.write_all(&idx.to_le_bytes())?;
        let us = &book.user_states[idx as usize];
        w.write_all(&us.net_qty.to_le_bytes())?;
        w.write_all(
            &us.order_count.to_le_bytes(),
        )?;
    }

    let free_count =
        book.user_free_list.len() as u32;
    w.write_all(&free_count.to_le_bytes())?;
    for &idx in &book.user_free_list {
        w.write_all(&idx.to_le_bytes())?;
    }

    Ok(())
}

/// Load orderbook from snapshot. Returns Box
/// to avoid stack overflow from event_buf.
pub fn load(
    r: &mut dyn Read,
) -> io::Result<Box<Orderbook>> {
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
            format!(
                "unsupported snapshot version {}",
                version,
            ),
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
    let compression =
        CompressionMap::new(mid_price, tick_size);

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
            format!(
                "slab bump {} exceeds capacity {}",
                bump, capacity,
            ),
        ));
    }
    let mut slab: Slab<OrderSlot> =
        Slab::new(capacity);
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
    let mut active_levels =
        vec![PriceLevel::default(); total];
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
    let user_bump = read_u16(r)?;
    let user_count = read_u32(r)?;
    let mut user_states =
        Vec::with_capacity(user_bump as usize);
    user_states.resize_with(
        user_bump as usize,
        UserState::default,
    );
    let mut user_map: FxHashMap<u32, u16> =
        FxHashMap::default();

    for _ in 0..user_count {
        let uid = read_u32(r)?;
        let idx = read_u16(r)?;
        let net_qty = read_i64(r)?;
        let order_count = read_u16(r)?;
        if idx as usize >= user_bump as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "user index out of bounds",
            ));
        }
        user_map.insert(uid, idx);
        let us = &mut user_states[idx as usize];
        us.user_id = uid;
        us.net_qty = net_qty;
        us.order_count = order_count;
    }

    let free_count = read_u32(r)?;
    let mut user_free_list =
        Vec::with_capacity(free_count as usize);
    for _ in 0..free_count {
        let fidx = read_u16(r)?;
        if (fidx as usize) >= user_bump as usize {
            eprintln!(
                "warn: snapshot user free-list \
                 index {} out of range {}, skipping",
                fidx, user_bump,
            );
            continue;
        }
        user_free_list.push(fidx);
    }

    // Box::new to heap-allocate immediately,
    // avoiding stack overflow from event_buf.
    let mut book = Box::new(
        Orderbook::new(config, capacity, mid_price),
    );
    book.active_levels = active_levels;
    book.orders = slab;
    book.best_bid_tick = best_bid_tick;
    book.best_ask_tick = best_ask_tick;
    book.sequence = sequence;
    book.user_states = user_states;
    book.user_map = user_map;
    book.user_free_list = user_free_list;
    book.user_bump = user_bump;
    Ok(book)
}

fn write_order(
    w: &mut dyn Write,
    s: &OrderSlot,
) -> io::Result<()> {
    w.write_all(&s.price.0.to_le_bytes())?;
    w.write_all(
        &s.remaining_qty.0.to_le_bytes(),
    )?;
    w.write_all(&[s.side])?;
    w.write_all(&[s.flags])?;
    w.write_all(&[s.tif])?;
    w.write_all(&s.next.to_le_bytes())?;
    w.write_all(&s.prev.to_le_bytes())?;
    w.write_all(&s.tick_index.to_le_bytes())?;
    w.write_all(&s.user_id.to_le_bytes())?;
    w.write_all(&s.sequence.to_le_bytes())?;
    w.write_all(
        &s.original_qty.0.to_le_bytes(),
    )?;
    w.write_all(&s.timestamp_ns.to_le_bytes())?;
    w.write_all(&s.order_id_hi.to_le_bytes())?;
    w.write_all(&s.order_id_lo.to_le_bytes())?;
    Ok(())
}

fn read_order(
    r: &mut dyn Read,
) -> io::Result<OrderSlot> {
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

fn write_level(
    w: &mut dyn Write,
    lvl: &PriceLevel,
) -> io::Result<()> {
    w.write_all(&lvl.head.to_le_bytes())?;
    w.write_all(&lvl.tail.to_le_bytes())?;
    w.write_all(&lvl.total_qty.to_le_bytes())?;
    w.write_all(&lvl.order_count.to_le_bytes())?;
    Ok(())
}

fn read_level(
    r: &mut dyn Read,
) -> io::Result<PriceLevel> {
    Ok(PriceLevel {
        head: read_u32(r)?,
        tail: read_u32(r)?,
        total_qty: read_i64(r)?,
        order_count: read_u32(r)?,
    })
}

fn read_u8(r: &mut dyn Read) -> io::Result<u8> {
    let mut buf = [0u8; 1];
    r.read_exact(&mut buf)?;
    Ok(buf[0])
}

fn read_u16(r: &mut dyn Read) -> io::Result<u16> {
    let mut buf = [0u8; 2];
    r.read_exact(&mut buf)?;
    Ok(u16::from_le_bytes(buf))
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
