use crate::book::BookConfig;
use crate::book::BookState;
use crate::book::Orderbook;
use crate::compression::CompressionMap;
use crate::level::PriceLevel;
use crate::order::OrderSlot;
use crate::order::ORDER_ACTIVE;
use crate::order::ORDER_REDUCE_ONLY;
use crate::slab::Slab;
use crate::user::UserRegistry;
use crate::user::UserState;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::Side;
use rsx_types::NONE;
use serde::Deserialize;
use serde::Serialize;
use std::io;
use std::io::Read;
use std::io::Write;

const SNAPSHOT_MAGIC: u32 = 0x5258_534E; // "RXSN"
const SNAPSHOT_VERSION: u32 = 3;
const SNAPSHOT_VERSION_WITH_DISPLAY_PRECISION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SnapshotU64(serde_bytes::ByteArray<8>);

impl SnapshotU64 {
    pub fn new(value: u64) -> Self {
        Self(serde_bytes::ByteArray::new(value.to_le_bytes()))
    }

    pub fn get(&self) -> u64 {
        u64::from_le_bytes(self.0.into_array())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookConfigSnapshotV1 {
    pub symbol_id: u32,
    pub tick_size: i64,
    pub lot_size: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderSnapshotV1 {
    pub handle: u32,
    pub price: i64,
    pub remaining_qty: i64,
    pub side: u8,
    pub tif: u8,
    pub reduce_only: bool,
    pub user_id: u32,
    pub sequence: u32,
    pub original_qty: i64,
    pub timestamp_ns: SnapshotU64,
    pub order_id_hi: SnapshotU64,
    pub order_id_lo: SnapshotU64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSnapshotV1 {
    pub index: u32,
    pub user_id: u32,
    pub net_qty: i64,
    pub order_count: u32,
    pub zero_since_ns: SnapshotU64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserRegistrySnapshotV1 {
    pub bump: u32,
    pub maintenance_cursor: u32,
    pub users: Vec<UserSnapshotV1>,
    pub free_indices: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookSnapshotV1 {
    pub sequence: SnapshotU64,
    pub config: BookConfigSnapshotV1,
    pub compression_mid_price: i64,
    pub slab_capacity: u32,
    pub slab_bump: u32,
    pub orders: Vec<OrderSnapshotV1>,
    pub free_handles: Vec<u32>,
    pub users: UserRegistrySnapshotV1,
}

/// Capture an owned logical checkpoint without borrowing the live book.
pub fn capture(book: &Orderbook) -> io::Result<BookSnapshotV1> {
    if book.state == BookState::Migrating {
        return Err(io::Error::other("cannot snapshot during migration"));
    }

    let bump = book.orders.len();
    let free_handles = book.orders.free_handles();
    let active_count = bump.saturating_sub(free_handles.len() as u32);
    let mut orders = Vec::with_capacity(active_count as usize);
    let mut visited = vec![false; bump as usize];

    for (tick, level) in book.active_levels.iter().enumerate() {
        let mut handle = level.head;
        let mut count = 0;
        let mut tail = NONE;
        while handle != NONE {
            if handle >= bump || visited[handle as usize] {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid order queue",
                ));
            }
            let slot = book.orders.get(handle);
            if !slot.is_active() || slot.tick_index != tick as u32 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "order queue disagrees with slab",
                ));
            }
            visited[handle as usize] = true;
            orders.push(OrderSnapshotV1 {
                handle,
                price: slot.price.0,
                remaining_qty: slot.remaining_qty.0,
                side: slot.side,
                tif: slot.tif,
                reduce_only: slot.is_reduce_only(),
                user_id: slot.user_id,
                sequence: slot.sequence,
                original_qty: slot.original_qty.0,
                timestamp_ns: SnapshotU64::new(slot.timestamp_ns),
                order_id_hi: SnapshotU64::new(slot.order_id_hi),
                order_id_lo: SnapshotU64::new(slot.order_id_lo),
            });
            count += 1;
            tail = handle;
            handle = slot.next;
        }
        if count != level.order_count || tail != level.tail {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "level metadata disagrees with queue",
            ));
        }
    }

    for handle in 0..bump {
        if book.orders.get(handle).is_active() != visited[handle as usize] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "slab disagrees with level queues",
            ));
        }
    }

    let mut users = Vec::with_capacity(book.users.user_map.len());
    for (&user_id, &index) in &book.users.user_map {
        let state = &book.users.user_states[index as usize];
        users.push(UserSnapshotV1 {
            index,
            user_id,
            net_qty: state.net_qty,
            order_count: state.order_count,
            zero_since_ns: SnapshotU64::new(state.zero_since_ns),
        });
    }
    users.sort_unstable_by_key(|user| user.index);

    Ok(BookSnapshotV1 {
        sequence: SnapshotU64::new(book.sequence),
        config: BookConfigSnapshotV1 {
            symbol_id: book.config.symbol_id,
            tick_size: book.config.tick_size,
            lot_size: book.config.lot_size,
        },
        compression_mid_price: book.compression.mid_price,
        slab_capacity: book.orders.capacity(),
        slab_bump: bump,
        orders,
        free_handles,
        users: UserRegistrySnapshotV1 {
            bump: book.users.user_bump,
            maintenance_cursor: book.users.maintenance_cursor,
            users,
            free_indices: book.users.user_free_list.clone(),
        },
    })
}

fn restore_users(snapshot: UserRegistrySnapshotV1) -> io::Result<UserRegistry> {
    let bump = snapshot.bump;
    let mut occupied = vec![false; bump as usize];
    let mut registry = UserRegistry::new();
    registry
        .user_states
        .resize_with(bump as usize, UserState::default);

    for user in snapshot.users {
        if user.index >= bump || occupied[user.index as usize] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid user registry index",
            ));
        }
        if registry.user_map.contains_key(&user.user_id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "duplicate user registry id",
            ));
        }
        occupied[user.index as usize] = true;
        registry.user_map.insert(user.user_id, user.index);
        registry.user_states[user.index as usize] = UserState {
            user_id: user.user_id,
            net_qty: user.net_qty,
            order_count: user.order_count,
            zero_since_ns: user.zero_since_ns.get(),
        };
    }

    let mut free = vec![false; bump as usize];
    for &index in &snapshot.free_indices {
        if index >= bump || occupied[index as usize] || free[index as usize] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid user registry free list",
            ));
        }
        free[index as usize] = true;
    }
    if occupied
        .iter()
        .zip(&free)
        .any(|(&is_occupied, &is_free)| !is_occupied && !is_free)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "user registry has untracked slots",
        ));
    }

    registry.user_free_list = snapshot.free_indices;
    registry.user_bump = bump;
    registry.maintenance_cursor = snapshot.maintenance_cursor;
    Ok(registry)
}

/// Restore a live book from the owned logical checkpoint.
pub fn restore(snapshot: BookSnapshotV1) -> io::Result<Box<Orderbook>> {
    if snapshot.slab_bump > snapshot.slab_capacity {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "slab bump exceeds capacity",
        ));
    }
    if snapshot.config.tick_size <= 0 || snapshot.config.lot_size <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid book alignment",
        ));
    }

    let bump = snapshot.slab_bump;
    let mut active = vec![false; bump as usize];
    let mut slab = Slab::new(snapshot.slab_capacity);
    slab.set_bump_next(bump);

    for order in &snapshot.orders {
        if order.handle >= bump || active[order.handle as usize] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid active order handle",
            ));
        }
        if order.side != Side::Buy as u8 && order.side != Side::Sell as u8 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid active order side",
            ));
        }
        active[order.handle as usize] = true;
        *slab.get_mut(order.handle) = OrderSlot {
            price: Price(order.price),
            remaining_qty: Qty(order.remaining_qty),
            side: order.side,
            flags: ORDER_ACTIVE
                | if order.reduce_only {
                    ORDER_REDUCE_ONLY
                } else {
                    0
                },
            tif: order.tif,
            _pad1: [0; 5],
            next: NONE,
            prev: NONE,
            tick_index: 0,
            _pad2: 0,
            user_id: order.user_id,
            sequence: order.sequence,
            original_qty: Qty(order.original_qty),
            timestamp_ns: order.timestamp_ns.get(),
            order_id_hi: order.order_id_hi.get(),
            order_id_lo: order.order_id_lo.get(),
            _pad4: [0; 24],
        };
    }

    let mut free = vec![false; bump as usize];
    for &handle in &snapshot.free_handles {
        if handle >= bump || active[handle as usize] || free[handle as usize] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid slab free list",
            ));
        }
        free[handle as usize] = true;
    }
    if active
        .iter()
        .zip(&free)
        .any(|(&is_active, &is_free)| !is_active && !is_free)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "slab has untracked slots",
        ));
    }
    slab.restore_free_handles(&snapshot.free_handles);

    let config = BookConfig {
        symbol_id: snapshot.config.symbol_id,
        tick_size: snapshot.config.tick_size,
        lot_size: snapshot.config.lot_size,
    };
    let mut book = Box::new(Orderbook::new(
        config,
        snapshot.slab_capacity,
        snapshot.compression_mid_price,
    ));
    book.orders = slab;
    book.users = restore_users(snapshot.users)?;

    for order in &snapshot.orders {
        let tick = book.compression.price_to_index(order.price);
        book.orders.get_mut(order.handle).tick_index = tick;
        let level = &mut book.active_levels[tick as usize];
        if level.order_count == 0 {
            level.head = order.handle;
        } else {
            let tail = level.tail;
            book.orders.get_mut(tail).next = order.handle;
            book.orders.get_mut(order.handle).prev = tail;
        }
        level.tail = order.handle;
        level.total_qty = level.total_qty.saturating_add(order.remaining_qty);
        level.order_count += 1;
    }

    book.sequence = snapshot.sequence.get();
    book.rebuild_occupancy();
    book.refresh_best_bid();
    book.refresh_best_ask();
    Ok(book)
}

/// Write the legacy RXSN v3 format.
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

    let free_handles = book.orders.free_handles();
    w.write_all(&(free_handles.len() as u32).to_le_bytes())?;
    for handle in free_handles {
        w.write_all(&handle.to_le_bytes())?;
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
    if version != SNAPSHOT_VERSION && version != SNAPSHOT_VERSION_WITH_DISPLAY_PRECISION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported snapshot version {}", version,),
        ));
    }
    let sequence = read_u64(r)?;

    // Config
    let symbol_id = read_u32(r)?;
    if version == SNAPSHOT_VERSION_WITH_DISPLAY_PRECISION {
        // RXSN v2 stores two display-precision bytes here.
        let _price_decimals = read_u8(r)?;
        let _qty_decimals = read_u8(r)?;
    }
    let tick_size = read_i64(r)?;
    let lot_size = read_i64(r)?;
    let config = BookConfig {
        symbol_id,
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

    let free_count = read_u32(r)?;
    if free_count > bump.saturating_sub(active_count) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "slab free-list count exceeds inactive slots",
        ));
    }
    let mut free_handles = Vec::with_capacity(free_count as usize);
    let mut seen = vec![false; bump as usize];
    for _ in 0..free_count {
        let handle = read_u32(r)?;
        if handle >= bump || slab.get(handle).is_active() || seen[handle as usize] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid slab free-list handle",
            ));
        }
        seen[handle as usize] = true;
        free_handles.push(handle);
    }
    if free_count != bump.saturating_sub(active_count) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "slab free-list does not cover inactive slots",
        ));
    }
    slab.restore_free_handles(&free_handles);

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
