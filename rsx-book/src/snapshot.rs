use crate::book::BookConfig;
use crate::book::BookState;
use crate::book::Orderbook;
use crate::order::OrderSlot;
use crate::order::ORDER_ACTIVE;
use crate::order::ORDER_REDUCE_ONLY;
use crate::user::UserRegistry;
use crate::user::UserState;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::Side;
use rsx_types::NONE;
use serde::Deserialize;
use serde::Serialize;
use std::io;

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

/// Restore a live book from the owned logical checkpoint, with the order
/// slab sized to the larger of `capacity` and the capacity it was written
/// with.
pub fn restore(snapshot: BookSnapshotV1, capacity: u32) -> io::Result<Box<Orderbook>> {
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

    let config = BookConfig {
        symbol_id: snapshot.config.symbol_id,
        tick_size: snapshot.config.tick_size,
        lot_size: snapshot.config.lot_size,
    };
    let mut book = Box::new(Orderbook::new(
        config,
        capacity.max(snapshot.slab_capacity),
        snapshot.compression_mid_price,
    ));
    let bump = snapshot.slab_bump;
    let mut active = vec![false; bump as usize];
    book.orders.set_bump_next(bump);

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
        *book.orders.get_mut(order.handle) = OrderSlot {
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
    book.orders.restore_free_handles(&snapshot.free_handles);
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
