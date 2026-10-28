# Price before time inside a compressed slot

Source: `specs/2/21-orderbook.md` § 2.6 Smooshed Ticks.

## Problem

A compressed slot (zones 1–4) holds several exact prices and both sides in one
arrival-ordered chain. A walk that fills every crossing maker it meets, oldest
first, lets an earlier worse price fill before a later better one when the
taker runs out inside the slot. The taker then pays more than the book offers,
and the better maker is skipped. Liquidation orders priced up to 9,999 bps
through the mark reach these zones.

## Fix

`match_at_level` leaves zone 0 alone: one price per slot, arrival order. In a
compressed slot `scan_crossing` sums the opposite-side makers that cross the
taker limit. If the taker covers that qty and the maker cap, every crossing
maker fills, so the order of the fills carries no meaning and the slot is
walked in arrival order. Otherwise the taker takes the best crossing price:
the same walk runs with the limit set to that price, which admits only that
price's makers in arrival order, and a rescan finds the next price. The walk
ends when the taker is filled or `MAX_MAKERS_PER_ORDER` makers are spent, so
the cap keeps the best prices. Replay runs the same code on the same orders and
rebuilds the same book.

## Cost

- Zone 0 pays one comparison of the slot index against the zone-0 edge per
  level.
- A compressed slot pays one scan of its chain per match, so a sweep that
  clears the slot costs O(k) for k orders. A taker that stops inside the slot
  pays one more scan per distinct price it fills.
- A later order priced better than the older ones in its slot fills ahead of
  them.
