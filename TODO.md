## CMP Protocol TODOs

- Don't send symbol_id in every CMP message. Instead,
  establish symbol during handshake or use a setup frame.
  symbol_id is per-stream, not per-message. Saves 4 bytes
  per record and simplifies wire format.
- Send events directly as WAL records (repr(C) structs)
  through the entire pipeline. No parsing/converting
  between 4 different type layers. Book Event → WAL record
  once, then WAL record flows unchanged to CMP, WAL, and
  downstream consumers.

## Gateway Runtime

- Move gateway away from monoio to a work-stealing
  runtime with io_uring support (tokio-uring, glommio,
  or nuclei). monoio is single-threaded per core with
  no work stealing -- fine for ME/risk tiles but gateway
  needs to handle many concurrent WS connections. A
  work-stealing runtime with io_uring keeps the latency
  benefits while distributing connection load across
  cores. Evaluate tokio + io_uring (via tokio-uring crate)
  first since the rest of the ecosystem (tracing, tower)
  already integrates with tokio.

## References

https://chessbr.medium.com/building-an-exchange-in-rust-part-1-78ab153864d7
https://github.com/MellowYarker/RustX
https://github.com/barter-rs/barter-rs
https://github.com/firedancer-io/firedancer

we'll be building an exchange in rust.

that is... exchange as the ones above BUT

- focus on speed ... monolythic core
- apis multiplexed outside in golang (LATER)
- the core is a single orderbook
- it has a tile architecture as does firedancer
- this basically means that the architecture is "actors" (as in Scala actor model) which share workload through spin-looping on a RTRB queue


Components

- user - has account
- symbol - has orderbook
- risk engine - each user has risk calculated from his positions on his account
- matching engine - each orderbook has one - orders coming in a queueu are matched
- stressing engine - testing data generation and profiling

Speed

- everything is optimized for speed from 1s
- monoio / or even custom http stack
- dataplane-userspace networking
- rtrc- busywaiting for work to split among cpus
- order routing - orderbooks are split among cores and servers by expected load
- proble is calculating risk as it needs data from all orderbooks possibly split and is slow
- but the internal system can gather positions and available margin along the way and deliver it to the matching engine prepared and fetched

Simulation

- latecy simulator... adding latency between components.

Latency Somulator

- generates client orders
- uses real prices form binance to bet for up down movement
- simulates 10 users
- with profiles mm, directional, yolo
- have account margins
- defunct users get removed and replaced
- saves a detailed latency trace from every part of the exchange and the client pov



Journeys


- its a pet exchange so there is an endpoint to create users - acoutns
- deposit funds - just changes account balance and withdraw funds
 - there is real api key auth ... api key generation, retrieval, for trading the auth is done using preauth tokens that expire.i  like 5min or so... this should be a speedup...

now the ji7rneys

create axcount

place order
cancel order 
modify order

v1 has only gtc orders, limit, no market

close account ... accounts are never truly deleted

isolated or portfoluo margin

user has a deposit in coins... spot coins.

then can trade perps,... ni spot tradings

isolated margin margined just by the spot coin itself.on the oerp book

portfolio margined by all coins... 


each coin has a param .. collateral ratio


Risk Engune

account has value . if value kess than 0 .. liquidated

has margin available to open positions
and margin used to maintain positions... 
if the latter is less than available collateral .. liquidate

collateral is coins deposited on spot with cillateral ratio applied.. 0.9 = just 0.9 times value

value is calclated based on an index price...  that is the mid of book.

it wikl eventually be a model and has to be calculatable 8ncrementalky by matching engine in each book update.

risk engine dies nit liquidate it only aggregates data on all orderbooks for the user - only the ones where he has spot deposits .. and provides info on collateral availability to the matching engine.

Order Speed

matching engine runs on a single core and thread... but

portfolio margin needs data from more books

this means potential latency.

to fix this matching engine maintains a list of collateral limits for every user inckuding his last known account state...  this is uodated when he trades by the data available... emg. the xurrent pruce of the coin and the risk engine sets periodic uodates when things chabge, also recrives updates from the other orderbooks. .. the xentealization is good because of the thirttking and non updates due to cirrelations and users hedgin across markets.

the matching engine then determines the users abikity to trade by the sane logic as the risk engine wiuld but using his margin estimates as oposed to exact numvers the risk ebgine sees...

liquidation is only triggered ny the risk engine

only record this direct trading as iptimizationfor marketmakers...  normal users go thriugh the risk engine

Scaling

risk engine across users

matching engine across symbols

Matching engine

receives user orsers and executes them

receives an order on behalf of user from risk engine over QUIC stream
and matches it 

Matchign

orders are inserted into the orderbook
and fills are generated

Speed

Network

use userspace networkign and speed optimized protocols

ingrews is json .. but this is a translationlayer translating to internal protocols

networking - see FUTURE.md for SMRB and transport layer optimizations

Datastructures

User balances and orderbook states.

access neest to be fast but oersistent monitored.  all events notified to the user must be persisted. ... this neds to be desinged carefukky befire implementation.

matching needs to be very fast as well as user lookup in the risk engine... optimize speed for frequent users with large volume... e.g some lru cache or so maybe prewarmed but as we can split risk engine by users we can safely keep most in memory it woyld seem

mathing engine data struxtures need to be optimized for no hash lookup, linear processing and good cache locality at all times


