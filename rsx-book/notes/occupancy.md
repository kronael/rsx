# Next occupied price: summary bitmap

An order book must find the next bid or ask after the current best price
empties. The compression map stores levels in a bounded array, but scanning
that array would still pay for empty slots.

## Problem

A sparse book can have a large gap between occupied prices. A linear scan
turns that gap into work on the match and touch-cancel paths. A tree avoids the
scan but duplicates the key structure already provided by the compression
map.

## Fix

Each side has a hierarchical bitmap over compression slots:

```text
price slots   [ ][ ][■][ ] ··· [■][ ][ ]
level 0 bits   0  0  1  0  ···  1  0  0
level 1       [       1       ][   1   ]
root          [           1           ]
```

Level 0 has one bit per price slot. Each higher level has one bit per word of
the level below. A summary bit is set when that lower word is non-empty.

For about 120,000 price slots, the hierarchy has three levels. Its words are
stored in contiguous vectors.

## Sawtooth ordering

Compression slots are not globally ordered by price. Each zone contains a
lower and upper price band, so crossing a zone boundary changes the physical
index order.

`price_asc` records at most ten index ranges in ascending price order. Ask
lookup walks those ranges forward and takes the first occupied bit. Bid lookup
walks them backward and takes the last occupied bit.

The bitmap is keyed by order side, not by whether an index falls above or
below the configured mid. A buy or sell can rest on either side of that mid.

## Maintenance

The occupancy bit and `PriceLevel::order_count` change together:

- the first order in a level sets its side's bit;
- removing the last order clears it;
- migration sets bits in the destination map;
- recentering starts with empty summaries;
- restore rebuilds summaries from captured logical orders.

A stale bit could skip liquidity or expose an empty level. Reference tests
compare bitmap lookup against a brute-force scan across random operations and
recentering.

## Cost removed

| operation | summary work |
|---|---|
| insert into an empty level | set one path through the bitmap |
| remove the last order | clear one path through the bitmap |
| find next best price | search one price-ordered range at a time through summary words |
| sweep `K` occupied levels | repeat the bounded lookup for each level consumed |

The hierarchy removes dependence on the number of empty slots between
occupied prices. A deep sweep remains linear in the number of levels it
actually consumes.

## Rejected alternatives

**A tree beside the level array** would store and compare keys already encoded
by the compression map. It would also add separately allocated nodes.

**A linked list of occupied levels** makes walking cheap but still needs an
index to find insertion neighbors and arbitrary crossing prices. The bitmap
provides that index directly.
