---
title: Closed-source high-performance messaging — what's out there, what we'd
       buy, what we'd be allowed to publish
date: 2026-05-24
status: verified
sources:
  - https://ultramessaging.github.io/ (Informatica, Ultra Messaging
    documentation root)
  - https://github.com/UltraMessaging/um_perf (Informatica,
    Ultra Messaging performance tools + reproduction guide)
  - https://www.informatica.com/products/data-integration/ultra-messaging.html
    (Informatica, UM product page; pricing is contact-sales)
  - https://29west.wordpress.com/tag/latency-busters-messaging-lbm/
    (29West blog, Todd Montgomery LBM era)
  - https://www.finextra.com/newsarticle/21207/informatica-acquires-low-latency-messaging-firm-29west
    (Finextra, 2010 Informatica acquires 29West)
  - https://marketswiki.com/wiki/29West (MarketsWiki, 29West
    founding and product history)
  - https://docs.tibco.com/products/tibco-rendezvous (TIBCO,
    Rendezvous documentation portal)
  - https://docs.tibco.com/pub/rendezvous/8.6.1/TIB_rv_8.6.1_relnotes.pdf
    (TIBCO, Rendezvous 8.6.1 release notes — PGM variant removed,
    rvrad deprecated)
  - http://tibcodose.blogspot.com/2016/04/tibco-rendezvous-rv-features-and-more.html
    (Tibcodose, RV protocol overview)
  - https://www.cl.cam.ac.uk/~jac22/out/msr-pgm.pdf (J. Crowcroft
    et al., Cambridge — The PGM Reliable Multicast Protocol)
  - http://www.tibco.com/products/automation/enterprise-messaging/ftl
    (TIBCO, FTL product page — 210 ns claim, 6M msg/s)
  - https://www.tibco.com/press-releases/2017/tibco-makes-enterprise-class-messaging-available-everyone
    (TIBCO, 2017 — FTL Community Edition announcement)
  - https://docs.tibco.com/pub/adfiles/7.0.0/license/TIB_adfiles_7.0.0_license.pdf
    (TIBCO, EULA — benchmark results as Confidential Information)
  - https://solace.com/products/performance/ (Solace, performance
    page with appliance latency claims)
  - https://www.arista.com/assets/data/pdf/solace-solarflare-arista.pdf
    (Arista/Solace/Solarflare 2010 whitepaper — 24-46 µs
    end-to-end at 1-5M msg/s)
  - https://solace.com/wp-content/uploads/2019/04/Solace-3560-Datasheet.pdf
    (Solace, 3560 appliance datasheet — FPGA, 20 µs at any rate)
  - https://solace.com/license-software/ (Solace, software EULA
    landing page)
  - https://solace.com/blog/introducing-vmr-community-edition/
    (Solace, VMR Community Edition — no license key, no time
    limit, renamed to PubSub+ Standard)
  - https://www.ibm.com/docs/en/wmllm/2.6/com.ibm.wllm.doc/api/javadoc/messaging/com/ibm/llm/rmm/common/RmmTierArbitratorParams.html
    (IBM, WebSphere MQ LLM 2.6 docs)
  - https://www-01.ibm.com/common/ssi/cgi-bin/ssialias?appname=USN&htmlfid=897%2FENUS217-109
    (IBM, 2017 announcement — withdrawing MQ LLM, Confinity LLM
    is the follow-on)
  - https://confinity-solutions.com/product/ (Confinity Solutions,
    CLLM product page — origin from IBM RMM, Nov 2007 debut)
  - https://docs.stacresearch.com/system/files/resource/files/STAC-Summit-4-May-2023-Confinity.pdf
    (STAC, Confinity LLM Summit deck May 2023)
  - https://docs.stacresearch.com/system/files/resource/files/GSL-Fall-2020-Confinity.pdf
    (STAC, GSL Fall 2020 Confinity CLLM 4.0 deck — FPGA, 2-4 µs)
  - https://stacresearch.com/news/2009/11/19/ibm-releases-first-stac-benchmarks-messaging-system
    (STAC, IBM WebSphere MQ LLM first STAC-M2 audit Nov 2009)
  - https://stacresearch.com/news/2012/02/23/juniper-networks-releases-stac-m2-benchmarks-using-qfabric-system
    (STAC, Juniper QFabric STAC-M2 — 10 µs mean, 1 µs σ)
  - https://stacresearch.com/m2 (STAC, M2 benchmark central)
  - https://stacresearch.com/benchmarks/ (STAC, benchmark
    catalogue including STAC-M2)
  - https://network.nvidia.com/pdf/whitepapers/Low-Latency-Solution-for-High-Frequency-Trading-from-IBM-and-Mellanox.pdf
    (Mellanox/IBM whitepaper — 4 µs at 100k, 6 µs at 1M msg/s on
    ConnectX-2 RoCE)
  - https://www.onixs.biz/ (OnixS, FIX Engine + venue handler
    SDK landing)
  - https://www.onixs.biz/cme-mdp-conflated-tcp-market-data-handler.html
    (OnixS, CME MDP Conflated UDP handler)
  - https://www.cmegroup.com/solutions/market-access/globex/develop-to-globex.html
    (CME, Develop to Globex)
  - https://cmegroupclientsite.atlassian.net/wiki/spaces/EPICSANDBOX/pages/714539039/iLink+Functional+Specification
    (CME, iLink Functional Specification — wiki, login-gated for
    full content)
  - https://www.cmegroup.com/globex/files/ilink-3-cgw-session-guidelines.pdf
    (CME, iLink 3 CGW session guidelines Dec 2023)
  - https://datamine.cmegroup.com/ (CME, DataMine pricing portal)
  - https://www.cmegroup.com/market-data/files/january-2025-market-data-fee-list.pdf
    (CME, January 2025 market data fee list)
  - https://www.nyse.com/connectivity/specs (NYSE, Pillar
    connectivity + protocol specs)
  - https://www.nyse.com/publicdocs/nyse/NYSE_Pillar_Gateway_FIX_Protocol_Specification.pdf
    (NYSE, Pillar Gateway FIX Protocol Spec)
  - https://www.nyse.com/publicdocs/nyse/NYSE_Pillar_Stream_Protocol_Specification.pdf
    (NYSE, Pillar Stream Protocol Spec)
  - https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/NQTVITCHSpecification.pdf
    (Nasdaq, TotalView-ITCH 5.0 spec)
  - https://www.nasdaqtrader.com/content/technicalsupport/specifications/tradingproducts/ouch4.2.pdf
    (Nasdaq, OUCH 4.2 spec)
  - https://wiki.wireshark.org/SoupBinTCP (Wireshark, SoupBinTCP
    overview)
  - https://aeron.io/docs/consulting/ (Real Logic, Aeron Premium
    feature list — DPDK, ATS, Cluster Standby)
  - https://aeron.io/premium-docs/aeron-dpdk/dpdk-overview.html
    (Real Logic, Aeron Premium DPDK overview)
  - https://aeron.io/aeron-premium/aeron-transport-kernel-bypass/
    (Real Logic, Aeron Premium kernel bypass — Solarflare/Mellanox,
    AWS Nitro support)
  - https://aws.amazon.com/blogs/industries/aeron-performance-enables-capital-markets-to-move-to-the-cloud-on-aws/
    (AWS Industries blog, Aeron on AWS — Premium DPDK 2M msg/s
    figure)
  - https://chronicle.software/fix-engine/ (Chronicle Software,
    Chronicle FIX product page — <4 µs round-trip)
  - https://www.endace.com/ (Endace, packet capture product
    family — relevant only as nanosecond-timestamp capture, not
    messaging)
  - https://www.databricks.com/blog/2021/11/08/eliminating-the-dewitt-clause-for-database-benchmarking.html
    (Databricks blog — DeWitt Embrace Clause, vendor list)
  - https://dwheeler.com/essays/dewitt-clause.html (David A.
    Wheeler, DeWitt clause as censorship — history)
  - https://danluu.com/anon-benchmark/ (Dan Luu, Oracle Ellison
    tried to have DeWitt fired — primary historical account)
  - https://www.brentozar.com/archive/2018/05/the-dewitt-clause-why-you-rarely-see-database-benchmarks/
    (Brent Ozar, DeWitt clause in modern SQL Server EULAs)
  - https://www.itprotoday.com/sql-server/devils-dewitt-clause
    (IT Pro Today, Microsoft SQL Server EULA quote — "may not
    disclose the results of any benchmark test")
  - https://en.wikipedia.org/wiki/David_DeWitt (Wikipedia, David
    DeWitt biography + Wisconsin Benchmark)
local_measurement: null
---

# Closed-source messaging — what's out there, what we'd buy, what we'd be allowed to publish

## TL;DR

| Product | Reliability | Public spec? | Free dev tier? | DeWitt-like clause? | What we can do |
|---|---|---|---|---|---|
| Informatica Ultra Messaging (LBM) | NAK over UDP multicast (LBT-RM) and unicast (LBT-RU) | API docs public; wire is not | No | Unconfirmed; EULA is private | Private bench under eval, no publish |
| TIBCO Rendezvous | PGM (NAK-based reliable multicast) or UDP best-effort | Protocol public via PGM RFC | License-ticket-free since 8.5.0 (2019), still paid | Yes — Confidential Information includes benchmark results (per TIBCO common EULA) | Implement from PGM RFC; cite RFC numbers |
| TIBCO FTL | Mixed: TCP, RDMA, UDP, shared memory; ACK + retransmit | Concepts public; wire not | Community Edition exists (2017+) | Yes — same TIBCO EULA family | Bench Community Edition privately; don't publish |
| Solace PubSub+ (software + appliance) | Per-protocol (SMF, MQTT, AMQP, JMS over TCP; appliance FPGA datapath) | SMF wire not public; AMQP/MQTT are open | PubSub+ Standard free, no time limit | Solace EULA — restrictions present, benchmark-publish language not located | Bench Standard edition; published latency from vendor whitepapers |
| Confinity LLM (ex IBM MQ LLM, ex 29West RMM/RUM heritage) | RMM (multicast NAK) + RUM (unicast ACK) | Vendor docs only | No | Unconfirmed | Cite STAC-published numbers; private eval if procured |
| OnixS SDKs | Wraps venue native protocols (CME, NYSE, Nasdaq, B3, Eurex, etc.) | Underlying venue specs are public; OnixS API is the value-add | Trial available; subscription model | Standard commercial SDK terms | Don't need: venue specs are public, can implement directly |
| CME iLink 3 + MDP 3 | SBE + FIXP over TCP (iLink) and UDP multicast (MDP) | iLink wiki public, full SBE templates via SFTP; MDP3 public | Yes — paper/cert env is free, prod feed is paid | Venue protocol, not a product EULA | Implement client from spec; bench against client itself |
| NYSE Pillar | TCP binary + FIX gateways; UDP multicast for marketdata | Yes — specs published openly on nyse.com | No prod, sim env via cert | Venue protocol | Same as CME — implement from spec |
| Nasdaq ITCH / OUCH / SoupBinTCP | SoupBinTCP frames sequenced (ACK + gap-fill) | Yes — full PDF specs on nasdaqtrader.com | UltraMessaging-tier sim envs, paid market data sub for live feed | Venue protocol | Same — implement from spec |
| Aeron Premium (Real Logic) | NAK over UDP + DPDK kernel bypass + ATS encryption + Cluster Standby | OSS Aeron protocol is public on github | OSS Aeron is Apache-2.0; Premium is commercial | Real Logic commercial-license terms not public | OSS Aeron benched already (`compare/aeron.md`); Premium adds ef_vi/DPDK transport — private eval only |
| Chronicle FIX | TCP FIX session with Chronicle Queue persistence | FIX is open; Chronicle's engine internals are not | Commercial only ("transparent licensing model") | Chronicle commercial-license terms not public | Vendor-published: <4 µs round-trip up to p99.9 — citable |
| STAC Research (org, not a product) | n/a — third-party benchmark auditor | STAC-M2 methodology is members-only; published reports are public | Reading published reports is free | n/a | **Cite STAC-audited reports freely**; that's the publication carve-out |
| Endace (out of scope) | Not messaging; nanosecond packet capture | n/a | Hardware appliance, no software-only tier | n/a | Not relevant to a CMP comparison; included only to disambiguate |

## The DeWitt clause: history + current state

### 1982: how this started

In 1982 David DeWitt, then an assistant professor at the University of
Wisconsin–Madison, ran a comparative benchmark (the **Wisconsin
Benchmark**, co-developed with Dina Bitton and Carolyn Turbyfill)
against multiple commercial databases. Oracle performed poorly.
Oracle's CEO Larry Ellison reportedly called Wisconsin's department
chair and asked that DeWitt be fired. The department refused. Ellison
then banned Oracle from hiring University of Wisconsin students, and —
durably — Oracle inserted a clause in its license agreement forbidding
any customer from publishing benchmark results without Oracle's prior
written approval.

The clause survived. Over the next two decades it became standard
boilerplate in commercial database EULAs: Microsoft SQL Server, IBM
DB2, Sybase, Informix, and many newer entrants copied it.
Microsoft's SQL Server EULA still contains the canonical form:

> "You may not disclose the results of any benchmark test of either
> the Server Software or Client Software to any third party without
> Microsoft's prior written approval."

(Quoted in IT Pro Today, citing the current SQL Server EULA.)

Sources: Dan Luu's primary account (`danluu.com/anon-benchmark`),
David Wheeler's essay (`dwheeler.com/essays/dewitt-clause.html`),
Wikipedia (`en.wikipedia.org/wiki/David_DeWitt`).

### Current state in 2026

- **Still standard** in: Oracle (database and middleware), Microsoft
  SQL Server, IBM DB2, Sybase, SAP HANA, Snowflake.
- **Dropped or inverted** by: Databricks (introduced a "DeWitt Embrace
  Clause" in 2021 — explicitly invalidates the other side's DeWitt
  clause if a competitor benchmarks Databricks); AWS and Azure
  service terms generally allow customer benchmarking and naming.

Source: Databricks blog post 2021-11-08.

### Application to messaging products

The clause is **less universally documented** for messaging
middleware than for databases, because messaging EULAs are typically
not publicly posted as PDFs. Where we could verify:

- **TIBCO**: the common TIBCO EULA template lists "Product or related
  performance test results derived by you, including but not limited
  to benchmark test results" as **Confidential Information**.
  Confidentiality bars third-party disclosure, which is functionally
  a DeWitt clause expressed as confidentiality rather than as a flat
  publication ban. Source: TIBCO ADfiles 7.0.0 EULA PDF.
- **Informatica Ultra Messaging**: the EULA is not publicly hosted.
  The `UltraMessaging/um_perf` repo's own README says results "can
  reliably be reproduced in their test lab" — which implies vendor
  permission for their own published numbers, not that customers can
  freely publish their own. Unconfirmed for customer-side publication.
- **Solace**: EULA is hosted publicly (`solace.com/license-software/`)
  but the binary PDF could not be parsed in research; standard
  Solace EULA references "audit provisions" and use restrictions.
  Benchmark-publish language not located in research. <!-- unverified:
  exact Solace benchmark-disclosure clause -->
- **Real Logic / Aeron Premium**: commercial license terms not
  publicly hosted. Unconfirmed.
- **Chronicle Software**: commercial license terms not publicly
  hosted. Unconfirmed. Note Chronicle does publish their own
  latency numbers freely.

### The STAC Research exception

[STAC Research](https://stacresearch.com/) is the financial-
industry-standard third-party benchmark auditor. The STAC Benchmark
Council includes 350+ banks and 50+ vendors. STAC-M2 is the messaging
middleware benchmark family.

STAC's value proposition to vendors is precisely that **STAC-audited
results may be publicly disclosed** — that's the carve-out from
DeWitt clauses. Vendors voluntarily submit to STAC audits, agree to
the methodology, and earn the right to put a STAC stamp on a
published latency number. Examples:

- IBM WebSphere MQ LLM — first STAC-M2 audit November 2009 (jitter
  σ ≤ 3 µs claimed).
- Juniper QFabric + messaging — 10 µs mean, 1 µs σ at baseline rate
  (STAC-M2, February 2012).
- Confinity CLLM 4.0 — FPGA-accelerated, 2-4 µs end-to-end,
  presented at STAC GSL Fall 2020.

For our purposes: **STAC-published numbers are freely citable** in
public documents like blog posts or specs, because STAC's contract
with vendors includes publication rights for audited results. They
cannot, however, be reproduced — the M2 test harness is
members-only.

## What "benchmarking" even means in this space

Three distinct activities, with very different legal status:

1. **Vendor self-publishes** — a vendor whitepaper claiming "we do
   N µs at M msg/s on hardware H". Always citable as "vendor X claims
   N µs". Quality varies; methodology is usually thin.
2. **STAC-audited public report** — vendor commissioned STAC, agreed
   to fixed methodology, got a stamp. Citable with the STAC report
   as source. Highest credibility short of our own measurement.
3. **End-user microbenchmark** — your shop bought a license,
   measured for internal capacity-planning. Almost always permitted
   *privately*. Almost always **forbidden to publish** without
   vendor written approval. This is the DeWitt zone.

For us at RSX, the practical implication is: we cannot run any of
these closed-source products and put numbers in `compare/`. We can:

- Cite vendor whitepapers (caveat the methodology).
- Cite STAC-audited reports (high confidence).
- Implement clients from public wire specs and bench our own client
  against itself (e.g. our own MDP3 decoder vs. our own MDP3 encoder).

Anything beyond that requires legal review.

## Products

### Informatica Ultra Messaging (UM / LBM)

#### What it is

The commercial descendant of 29West **Latency Busters Messaging**
(LBM). Founded 2002 by Mark Mahowald and Todd Montgomery; Montgomery
joined as senior architect in 2004 and the same year delivered the
first LBM release. Informatica acquired 29West in 2010. The product
is still sold as **Informatica Ultra Messaging** as of 2026.

This is the closest commercial peer to CMP — same family lineage
(Montgomery later co-created Aeron with Martin Thompson at Real
Logic). LBM was the production benchmark that Aeron, and via Aeron
our CMP NAK design, descended from.

#### Reliability mechanism

NAK-based reliable transport over UDP. Two primary modes:

- **LBT-RM** (Reliable Multicast): source sends to a multicast group;
  receivers detect gap by seq number, send NAK, source retransmits
  from in-memory transmission window. NAK suppression eliminates
  redundant retransmits.
- **LBT-RU** (Reliable Unicast): point-to-point, same NAK model.
- **SMX** (Shared Memory Transport): sub-100-ns latency between
  processes on the same host.

Same primitive as Aeron and CMP.

#### Wire format

Internal. The Ultra Messaging documentation site
(`ultramessaging.github.io`) documents the **API** in detail but
not the on-wire format. There is no public Wireshark dissector.
Customers and partners receive wire docs under NDA.

#### License + cost

Closed source. Pricing is contact-sales. The Informatica product
page has a "drop us an email" call to action. No public list price.
General Informatica enterprise software is in the
$100k-300k-per-processor range; UM is presumed comparable but
unverified.

#### DeWitt / publishing

Customer EULA not public. The `UltraMessaging/um_perf` GitHub
repo publishes Informatica's *own* benchmark results, framed as
"users are strongly advised to perform the same tests on hardware
that is as close as possible to their anticipated production
hardware" — i.e. vendor-published reference numbers with
reproduction instructions, but the customer's own results almost
certainly fall under standard confidentiality terms. <!-- unverified:
customer publication rights specifically -->

#### Published numbers (vendor)

From `um_perf` README and product brochure:

| Mode | Throughput |
|---|---:|
| Streaming, no persistence | 1.4M msg/s |
| Single SPP Store (persistence) | 550k msg/s |
| Single RPP Store | 760k msg/s |
| 3-store quorum/consensus | 740k msg/s |
| Load-balanced RPP (3 sources) | 1M msg/s |
| Batched | up to 1.5M msg/s |
| SMX (intra-host shared memory) | sub-100 ns p50 |

Public academic literature has cited LBM at ~1-5 µs one-way latency
on 10 GbE — but those numbers are old and re-citation depends on
hardware era.

#### What we can do

- **Acquire eval license** (process: contact Informatica sales,
  argue for a 30-90 day evaluation, expect significant friction
  and legal review). Bench privately. Do not publish numbers.
- **Cite their own published numbers** from `um_perf` and the
  brochure, with attribution.
- **Read the API docs** (publicly hosted) to understand their
  design vocabulary and inform our own. The public docs are
  high quality.
- **Do not implement an LBT-RM compatible client** — the wire is
  closed and reverse-engineering it would invite legal exposure.

#### Sources

- https://ultramessaging.github.io/
- https://github.com/UltraMessaging/um_perf
- https://www.informatica.com/products/data-integration/ultra-messaging.html
- https://www.finextra.com/newsarticle/21207/informatica-acquires-low-latency-messaging-firm-29west
- https://marketswiki.com/wiki/29West

---

### TIBCO Rendezvous (TIB/RV)

#### What it is

TIBCO Rendezvous, originally **Teknekron Information Bus**
(rebranded TIB/RV in 1997 after the TIBCO spin-out), is the
oldest still-shipping commercial message bus. Subject-based
publish/subscribe over IP multicast. Pervasive in trading firms
through the 1990s-2010s; still deployed but TIBCO recommends FTL
for new builds.

Still actively maintained — latest release 8.6.1 (November 2022).
Software activation no longer requires license tickets since 8.5.0
(December 2019).

#### Reliability mechanism

Two transports:

- **TIBCO PGM variant** — historically supported, removed as of 8.6.x.
- **TRDP** (TIBCO Reliable Datagram Protocol) — TIBCO's proprietary
  reliable UDP multicast; conceptually similar to PGM (NAK-based)
  but the wire is closed.
- **UDP** best-effort, for non-critical subjects.

PGM itself (RFC 3208) is a published reliable multicast protocol
that uses NAKs, NCFs (NAK Confirmations), and RDATA (Repair Data)
sent up a tree of designated retransmit points. The Cambridge
paper by Crowcroft et al. is a good reference for PGM semantics.

#### Wire format

PGM is publicly specified in RFC 3208. TRDP (the post-PGM-removal
variant) is internal. TIBCO's documentation describes the
**rvd** daemon architecture and `tibrv` API but not on-wire bytes.

#### License + cost

Closed source. Per-process license model (one rvd daemon per host).
Concrete pricing is contact-sales; older deployments commonly
priced in the low tens of thousands of dollars per server per year.

#### DeWitt / publishing

TIBCO's common EULA defines "Confidential Information" to include
"Product or related performance test results derived by you,
including but not limited to benchmark test results". That's a
DeWitt clause expressed as confidentiality. Source: TIBCO ADfiles
7.0.0 EULA PDF (the language is copy-pasted across TIBCO product
EULAs in the docs.tibco.com archive).

#### Published numbers

Vendor doesn't publish a current public latency number for RV.
Historical industry reports placed RV at 50-500 µs end-to-end
depending on subject-tree complexity, vs LBM at <10 µs — which is
why TIBCO built FTL.

#### What we can do

- **Implement an rvd-compatible client over PGM**: PGM is RFC-
  specified, NAK semantics are documented. We could build a client
  that talks to existing rvd brokers if we needed regulatory replay
  of historical TIBCO traffic. Not a near-term need.
- **Cite RFC 3208** for the PGM reliability mechanism in compare/
  docs — it's the open spec that influenced the same lineage.
- **Do not bench RV directly** — the license terms make it
  unattractive and the product is mid-deprecation.

#### Sources

- https://docs.tibco.com/products/tibco-rendezvous
- https://docs.tibco.com/pub/rendezvous/8.6.1/TIB_rv_8.6.1_relnotes.pdf
- http://tibcodose.blogspot.com/2016/04/tibco-rendezvous-rv-features-and-more.html
- https://www.cl.cam.ac.uk/~jac22/out/msr-pgm.pdf

---

### TIBCO FTL

#### What it is

TIBCO's modern successor to Rendezvous; positioned as the
ultra-low-latency offering. Heterogeneous transport — same API
backed by TCP, UDP unicast/multicast, RDMA (InfiniBand /
RoCE), or shared memory. Topic-based pub/sub with content-based
filtering.

Free **Community Edition** since 2017 for dev/test and entry-level
production.

#### Reliability mechanism

Per-transport. TCP gives ACK-based reliability. The RDMA transport
uses InfiniBand reliable-connected semantics. UDP multicast uses
TIBCO's own NAK + retransmit window (similar in shape to TRDP).

#### Wire format

Internal. The Concepts Guide on `docs.tibco.com` describes the
architecture and API; wire-level bytes are not published.

#### License + cost

Community Edition: free, restricted scale.
Enterprise: contact-sales. TrustRadius pricing data point: customer
mentions ranged from ~$10k for small dev to >$500k for full
enterprise deployment. Order-of-magnitude only.

#### DeWitt / publishing

Same TIBCO EULA family as Rendezvous — benchmark results treated
as Confidential Information. Functional DeWitt clause.

#### Published numbers (vendor)

From the TIBCO FTL product page:

> "very low latency (as low as 210 nanoseconds) and can deliver
> over 6 million messages per second with a single instance"

The 210 ns claim is shared-memory transport intra-host (analogous
to LBM SMX). Network transports are µs-range and vendor doesn't
publish those specifically on the marketing page.

No STAC-M2 audit on the public STAC site for FTL (verified by
searching `stacresearch.com` results 2026-05-24).
<!-- unverified: whether there's a private/Vault STAC report -->

#### What we can do

- **Bench Community Edition privately** to compare our CMP send
  latency vs FTL UDP unicast on the same hardware. Do not publish.
- **Cite the vendor's own published 210 ns / 6M msg/s number** with
  attribution + caveat that it's vendor-marketing-grade methodology.
- **Use the public architecture docs** to understand which
  reliability modes a comparable commercial product supports.

#### Sources

- http://www.tibco.com/products/automation/enterprise-messaging/ftl
- https://www.tibco.com/press-releases/2017/tibco-makes-enterprise-class-messaging-available-everyone
- https://docs.tibco.com/products/tibco-ftl-enterprise-edition-7-1-1
- https://docs.tibco.com/pub/adfiles/7.0.0/license/TIB_adfiles_7.0.0_license.pdf

---

### Solace PubSub+

#### What it is

Broker-based event mesh. Three form factors:

- **PubSub+ Standard** — free software broker, no license key
  required, no time limit. Best-effort community support.
- **PubSub+ Enterprise** — paid software broker; 90-day eval via
  free product key.
- **PubSub+ Appliance (3530, 3560, etc.)** — purpose-built
  hardware with FPGA datapath and network processors. No OS in
  the data path.

Unlike Aeron / LBM / CMP, Solace is **broker-centric** — publishers
and subscribers connect to a central broker rather than peering.
That's a different design point but worth including as the most
commonly benchmarked commercial event broker.

#### Reliability mechanism

Per-protocol — Solace's brokers speak SMF (Solace Message Format,
proprietary), AMQP, MQTT, JMS, and REST. Each uses its own ACK
semantics over TCP. The FPGA appliances do not change reliability
semantics, only the datapath speed.

#### Wire format

SMF is internal. AMQP, MQTT, JMS, REST are public standards that
Solace also speaks.

#### License + cost

- Standard: free.
- Enterprise: contact-sales.
- Appliance: hardware, contact-sales. Industry reports place 3560
  appliances in the $100k+ range, unverified.

#### DeWitt / publishing

Solace software EULA is at `solace.com/license-software/`. The
publicly hosted PDF includes audit provisions and standard use
restrictions (no reverse engineering, no redistribution).
Benchmark-publication language was not located in research —
**unverified**. <!-- unverified: Solace EULA benchmark clause
specifically -->

#### Published numbers (vendor)

Vendor whitepapers (Arista/Solace/Solarflare 2010, current
`solace.com/products/performance/` page):

| Configuration | Throughput | p50 latency | p99.9 latency |
|---|---:|---:|---:|
| 3560 appliance, FPGA datapath, 1M msg/s | 1M msg/s | 24 µs | 27 µs |
| 3560 appliance, 5M msg/s | 5M msg/s | 35 µs | 46 µs |
| 3560 appliance, "any rate" claim | any | ~20 µs | — |

Software broker numbers are not on the same public marketing page;
appliance-only.

#### What we can do

- **Bench PubSub+ Standard locally**, free, no time limit. Most
  faithful comparison would be Solace SMF over loopback vs our
  CMP over loopback. Internal use only.
- **Cite the appliance whitepaper numbers** with attribution.
- **Read the AMQP/MQTT support docs** for free interoperability
  testing if we ever needed a third-party client to drive Solace.

#### Sources

- https://solace.com/products/performance/
- https://www.arista.com/assets/data/pdf/solace-solarflare-arista.pdf
- https://solace.com/wp-content/uploads/2019/04/Solace-3560-Datasheet.pdf
- https://solace.com/license-software/
- https://solace.com/blog/introducing-vmr-community-edition/

---

### IBM MQ Low Latency Messaging (LLM) → Confinity LLM (CLLM)

#### What it is — and the lineage

This is one product with three names over fifteen years:

1. **29West RMM/RUM** (Reliable Multicast / Unicast Messaging) — the
   29West technology after Informatica acquired the LBM brand.
   (The RMM/RUM IP, separate from the LBM product, was originally
   developed at IBM Research — the lineage cross-pollinates.)
2. **IBM WebSphere MQ Low Latency Messaging (MQ LLM)** — debuted
   November 2007, built on the RMM/RUM core.
3. **Confinity LLM (CLLM)** — IBM withdrew MQ LLM from market in
   July 2017 (IBM announcement 217-109). Confinity Solutions GmbH
   took over the product and rebranded it CLLM. Existing IBM
   customers were pointed at Confinity as the supported follow-on.

Confinity is a small German company that picked up the codebase
and now sells CLLM + an FPGA-accelerated variant (CLLM 4.0 on AMD
Alveo).

#### Reliability mechanism

- **RMM** (multicast): NAK-based, similar to LBT-RM and PGM.
- **RUM** (unicast): ACK + retransmit, similar to TCP without
  congestion control.

#### Wire format

Internal. No public Wireshark dissector. STAC presentations show
that the protocol can run over IB (RDMA), 10/40/100 GbE, RoCE, and
shared memory.

#### License + cost

Closed source, contact-sales. IBM's historical MQ LLM pricing was
in the "enterprise tier" with named-user and PVU models; Confinity
pricing is contact-sales only.

#### DeWitt / publishing

IBM software EULAs historically contained benchmark-restriction
clauses (the standard "International Program License Agreement"
template). Confinity's EULA is not publicly hosted. Unconfirmed
in detail. <!-- unverified: Confinity-specific clause -->

The exception, again: STAC audits. Both IBM (2009 onward) and
Confinity (2020 onward) have published STAC-audited reports —
those numbers are citable.

#### Published numbers

| Source | Configuration | Numbers |
|---|---|---|
| STAC-M2 / IBM, Nov 2009 | first MQ LLM audit | σ ≤ 3 µs |
| Mellanox/IBM whitepaper, ConnectX-2 + RoCE | MQ LLM, 100k msg/s | 4 µs mean |
| Mellanox/IBM whitepaper, ConnectX-2 + RoCE | MQ LLM, 1M msg/s | 6 µs mean |
| STAC GSL Fall 2020 | Confinity CLLM 4.0 (FPGA on AMD Alveo) | 2-4 µs end-to-end |

#### What we can do

- **Cite the STAC reports** — they're public and audited.
- **Reference the IBM/Mellanox whitepaper numbers** — vendor
  marketing-grade but with disclosed methodology.
- **Not pursue a private bench** — Confinity is a small vendor, an
  eval would be a major procurement event for a comparison that
  the STAC numbers already document.

#### Sources

- https://www-01.ibm.com/common/ssi/cgi-bin/ssialias?appname=USN&htmlfid=897%2FENUS217-109
- https://confinity-solutions.com/product/
- https://docs.stacresearch.com/system/files/resource/files/STAC-Summit-4-May-2023-Confinity.pdf
- https://docs.stacresearch.com/system/files/resource/files/GSL-Fall-2020-Confinity.pdf
- https://network.nvidia.com/pdf/whitepapers/Low-Latency-Solution-for-High-Frequency-Trading-from-IBM-and-Mellanox.pdf
- https://stacresearch.com/news/2009/11/19/ibm-releases-first-stac-benchmarks-messaging-system

---

### OnixS FIX/UDP libraries

#### What it is

A library vendor, not a transport vendor. OnixS sells C++/C#/Java
SDKs that implement the **client side** of specific venue
protocols — CME MDP3, CME iLink 3, NYSE Pillar, Nasdaq ITCH,
Eurex ETI, B3 UMDF, etc. The underlying wire is the public venue
spec; the OnixS value-add is a tuned, certified, supported client.

#### Reliability mechanism

Inherited from the venue protocol the SDK wraps. For UDP market
data SDKs: gap detection + retransmit request via the venue's
TCP recovery channel. For FIX/SBE order entry: ACK + sequence
numbers per the venue's session protocol.

#### Wire format

Public — the SDKs implement publicly published venue specs.

#### License + cost

Subscription model: "inclusive of license, maintenance support,
upgrades and updates to the software, the industry protocol
standards, and the venue specific API implementations". Per-
venue, per-environment pricing; contact-sales. Industry reports
suggest $20k-150k per venue per year, unverified.

#### DeWitt / publishing

Standard commercial-SDK EULA. Specific benchmark-publication
language not located. Unverified, but the underlying *venue
protocol* is public so anyone can implement and bench their own
client.

#### Published numbers

OnixS occasionally publishes p50/p99 figures in venue-specific
blog posts but no STAC-audited messaging report on the public
STAC site as of 2026-05-24.
<!-- unverified: STAC private/Vault submissions by OnixS -->

#### What we can do

- **Implement venue clients ourselves** from the public spec.
  This is what `rsx-messages` already does for CMP framing —
  we can do the same for CME MDP3 or Nasdaq ITCH if we ever
  need to compare wire-format encoding/decoding cost.
- **Don't pay OnixS unless we need certification** — a venue
  certification process treats an OnixS-licensed client as a
  shortcut. For benchmarking, irrelevant.

#### Sources

- https://www.onixs.biz/
- https://www.onixs.biz/cme-mdp-conflated-tcp-market-data-handler.html

---

### CME iLink 3 + MDP 3

#### What it is

Two CME Globex protocols:

- **iLink 3**: order entry. Simple Binary Encoding (SBE) messages
  framed in FIX Performance (FIXP) session layer over TCP. Replaced
  the older iLink 2 (FIX over TCP) for performance reasons.
- **MDP 3** (Market Data Platform 3.0): market data dissemination.
  SBE-encoded incremental updates over UDP multicast, with
  separate A/B feeds for redundancy and a TCP replay channel for
  gap recovery.

These are the canonical "modern futures-exchange protocols".

#### Reliability mechanism

- iLink 3: FIXP session layer = ACK + sequenced gap recovery over
  TCP. No NAKs (TCP handles transport).
- MDP 3: A/B feed redundancy + per-symbol sequence numbers +
  on-demand replay channel for gaps. Receiver detects gaps,
  requests retransmit via TCP.

#### Wire format

Public. CME publishes:

- The iLink 3 Functional Specification (wiki, on
  `cmegroupclientsite.atlassian.net`, partial public access).
- The iLink 3 CGW Session Guidelines (PDF, fully public, December
  2023 latest).
- The SBE template XML files (via `ftp://ftp.cmegroup.com/SBEFix/`).
- The MDP 3.0 specification.

OnixS, Nanoconda, B2BITS, and others have published implementation
guides that flesh out the protocol publicly.

#### License + cost

The **protocol** is free to implement. The **certification env**
(CME New Release / autocert+) is free. The **production
connectivity** requires a CME-approved colo presence and exchange
fees. The **historical/live market data** is paid: CME DataMine
charges per-dataset per-month with discounts for 12-month prepay;
educational use gets 50% off. The CME market data fee list
(`january-2025-market-data-fee-list.pdf`) is hundreds of pages of
SKUs.

For a benchmark-only use case (encode/decode latency only, no
live feed), you can implement the SBE templates from the public
XML and use synthetic message generators.

#### DeWitt / publishing

This is a venue protocol, not a vendor product. CME publishes its
own latency claims (e.g. exchange match latency); customers can
publish their own client latency numbers freely. There is no
DeWitt-style clause on the protocol specification itself.

#### Published numbers

CME publishes per-product matching latency in their press
releases. iLink 3 was designed to be faster than iLink 2; specific
numbers depend on which match engine and round-trip endpoint
you're measuring.

For client-side: any of the SBE benchmarks in the public domain
show sub-microsecond encode/decode for typical message sizes
(consistent with our own SBE-style fixed-record format).

#### What we can do

- **Implement an MDP3 decoder** from the public spec. Bench
  our decoder against our encoder. Compare to `rsx-messages`
  CMP framing.
- **Compare wire formats**: MDP3 SBE message layout vs our
  16B header + repr(C, align(64)) payload. Both are zero-copy
  binary; the comparison is informative.
- **Cite published CME latency numbers** in `compare/` docs
  with attribution.
- **Do NOT subscribe to DataMine** for benchmarking — not needed
  unless we want to replay real historical traffic shape.

#### Sources

- https://www.cmegroup.com/solutions/market-access/globex/develop-to-globex.html
- https://cmegroupclientsite.atlassian.net/wiki/spaces/EPICSANDBOX/pages/714539039/iLink+Functional+Specification
- https://www.cmegroup.com/globex/files/ilink-3-cgw-session-guidelines.pdf
- https://datamine.cmegroup.com/
- https://www.cmegroup.com/market-data/files/january-2025-market-data-fee-list.pdf

---

### NYSE Pillar

#### What it is

NYSE Group's integrated trading platform spanning NYSE, NYSE
Arca, NYSE American (equities and options), NYSE National, NYSE
Texas. Single protocol family across markets.

Three protocol surfaces:

- **Pillar Gateway Binary**: native binary order entry over TCP.
- **Pillar Gateway FIX**: FIX 4.4 over TCP with NYSE custom tags.
- **Pillar Stream**: market data; TCP login + UDP multicast for
  the actual data, similar shape to CME MDP3.

#### Reliability mechanism

- Order entry (Binary or FIX): TCP, sequenced. Unsequenced control
  messages for login etc.
- Market data: UDP multicast incremental updates + TCP replay /
  snapshot channel for gap recovery.

#### Wire format

Public. NYSE hosts the specifications openly at
`nyse.com/publicdocs/nyse/`. The Binary Gateway Spec, FIX Gateway
Spec, Stream Protocol Spec, and Floor Broker FIX Spec are all
PDFs anyone can download.

#### License + cost

Same model as CME — protocol is free, certification env is free,
production connectivity requires NYSE colo and trading fees,
market data feeds are paid subscriptions through NYSE Market Data.

#### DeWitt / publishing

Venue protocol; no DeWitt clause on the spec itself.

#### Published numbers

NYSE published latency improvements through Pillar migrations
(e.g. "92% improvement in 99th percentile" claim during migration
press releases). Concrete µs figures from NYSE itself are
intermittent; third-party measurements are more reliable.

#### What we can do

- Implement Pillar Stream decoder from the spec.
- Same value as MDP3: wire-format comparison data point.

#### Sources

- https://www.nyse.com/connectivity/specs
- https://www.nyse.com/publicdocs/nyse/NYSE_Pillar_Gateway_FIX_Protocol_Specification.pdf
- https://www.nyse.com/publicdocs/nyse/NYSE_Pillar_Stream_Protocol_Specification.pdf

---

### Nasdaq ITCH / OUCH / SoupBinTCP

#### What it is

Three layered protocols:

- **SoupBinTCP**: the session layer. Framing, authentication,
  heartbeats, sequencing, gap recovery. Application-protocol-
  agnostic.
- **OUCH**: native order entry protocol, runs on top of
  SoupBinTCP.
- **ITCH** (TotalView-ITCH): market data dissemination. Available
  in three variants — UDP multicast, MoldUDP64 (also UDP multicast
  with sequencing + gap-fill), and SoupBinTCP unicast.

This is the canonical "Nasdaq family" referenced in HFT design
discussions. SoupBinTCP has been studied and reverse-implemented
exhaustively; Wireshark has a full dissector.

#### Reliability mechanism

- SoupBinTCP: TCP underneath; SoupBin adds sequence numbers and
  optional gap-fill semantics on top.
- MoldUDP64: UDP multicast with sequence numbers; gap recovery via
  a separate retransmit-request channel.
- ITCH over multicast: best-effort UDP, gap recovery via TCP.

#### Wire format

Fully public. Nasdaq publishes the OUCH 4.2 PDF, the TotalView-
ITCH 5.0 PDF, and the SoupBinTCP overview is on the Wireshark
wiki + several third-party guides.

The MoldUDP64 spec is also public — that's an interesting
comparison point for CMP since it's UDP-multicast-with-sequence-
numbers-and-NAK, conceptually the same primitive.

#### License + cost

Free to implement. Live market data requires a Nasdaq market
data subscription (per-user fees, contract with Nasdaq Global Data
Products). Cert env is free.

Databento and similar third-party resellers offer historical
TotalView-ITCH access without going through Nasdaq directly.

#### DeWitt / publishing

Venue protocol; no DeWitt clause on the spec.

#### Published numbers

Nasdaq's own published latency for OUCH order entry is in the
"single-digit microseconds" range to/from their match engine.
Public per-message decode benchmarks for ITCH are common in HFT
literature (sub-microsecond per message for tight C++/Rust
implementations).

#### What we can do

- **Implement an ITCH 5.0 decoder** in Rust. The protocol is small
  (a few dozen message types, all fixed-layout binary). Compare
  encode/decode cost with `rsx-messages`.
- **Implement MoldUDP64** as a CMP analog — same wire shape
  (UDP multicast + sequence numbers + NAK) but with a published
  spec. Useful as a reference implementation we can bench against
  our own CMP.

#### Sources

- https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/NQTVITCHSpecification.pdf
- https://www.nasdaqtrader.com/content/technicalsupport/specifications/tradingproducts/ouch4.2.pdf
- https://wiki.wireshark.org/SoupBinTCP

---

### Aeron Premium (Real Logic commercial tier)

#### What it is

Open-source Aeron (Apache-2.0, github `aeron-io/aeron`) is already
covered in `rsx-dxs/compare/aeron.md`. **Aeron Premium** is the
commercial tier sold by Real Logic, the company founded by Martin
Thompson and Todd Montgomery that maintains Aeron. Premium adds:

- **DPDK transport** — kernel bypass via DPDK PMD, optimized for
  Solarflare/Xilinx and NVidia/Mellanox NICs (incl. AWS Nitro).
- **Aeron Transport Security (ATS)** — encrypted transport with
  claimed minimal latency overhead.
- **Aeron Cluster Standby** — warm DR + async snapshots for the
  Raft cluster layer.
- Commercial support contract.

#### Reliability mechanism

Identical to OSS Aeron — NAK-based reliable UDP, term-buffer
log, multicast support via PGM-style NAK suppression. Premium
swaps the BSD socket transport for DPDK; semantics unchanged.

#### Wire format

Identical to OSS Aeron, which is fully open on github. The
wire is the same; only the I/O layer underneath differs.

#### License + cost

Commercial, contact-sales (`sales@aeron.io`). No public price
list. Term licensing model; multi-environment per-host pricing
inferred from sales conversation reports.

#### DeWitt / publishing

Commercial license terms not publicly hosted. Unconfirmed.
<!-- unverified: Aeron Premium EULA benchmark clause -->

In practice Real Logic publishes their own benchmark numbers
freely (AWS blog, conference talks).

#### Published numbers (vendor)

From the AWS Industries blog and the Aeron Premium kernel-bypass
page:

- **OSS Aeron**: ~250k msg/s p99 < 100 µs on AWS ENA (our own
  measurement gave p50 ~21 µs).
- **Aeron Premium**: >2,000,000 msg/s — claimed 8× improvement
  over OSS via DPDK.
- Per-message latency on DPDK with Solarflare ef_vi: sub-µs
  one-way claimed in conference talks, not in a formal whitepaper.

#### What we can do

- **OSS Aeron is already benched** in `rsx-dxs/compare/aeron.md`.
- **Aeron Premium**: acquire eval, bench DPDK vs our own CMP-over-
  DPDK path if we ever build one. Don't publish numbers.
- **Cite the AWS blog's 2M msg/s** figure with attribution.

#### Sources

- https://aeron.io/docs/consulting/
- https://aeron.io/premium-docs/aeron-dpdk/dpdk-overview.html
- https://aeron.io/aeron-premium/aeron-transport-kernel-bypass/
- https://aws.amazon.com/blogs/industries/aeron-performance-enables-capital-markets-to-move-to-the-cloud-on-aws/

---

### Chronicle FIX (Chronicle Software)

#### What it is

Java FIX engine library with Chronicle Queue (off-heap persistent
log) underneath. Targets sell-side and exchange FIX gateways.
Single-threaded design, off-heap allocation, no GC pause on hot
path.

Not a transport — sits *above* TCP and provides the FIX session
layer + persistence.

#### Reliability mechanism

FIX session layer over TCP. Chronicle's value-add is
deterministic latency: no Java GC pause, off-heap message storage
via Chronicle Queue.

#### Wire format

FIX 4.x / FIX 5 / FIXT 1.1 — fully public industry standards
maintained by FIX Trading Community.

#### License + cost

Commercial. "Simple and transparent licensing model" per their
marketing site — actual numbers contact-sales. Customer testimonials
cite "8 of the top 11 investment banks" as customers, suggesting
enterprise pricing.

#### DeWitt / publishing

Commercial license terms not publicly hosted. Unverified.
<!-- unverified: Chronicle FIX EULA benchmark clause -->

Chronicle publishes their own latency figures openly:

> "handles a New Order Single round-trip in less than 4 µs up to
> the 99.9th percentile"
> "smart field ordering allows messages to be generated and parsed
> in less than 1 µs at high percentiles"

#### Published numbers (vendor)

- Round-trip NewOrderSingle: <4 µs p99.9.
- FIX encode/decode: <1 µs at high percentiles.

#### What we can do

- **Cite the vendor numbers** with attribution.
- **Bench Chronicle Queue's open-source portion** — Chronicle
  Queue itself is Apache-2.0 and is benched in
  `rsx-dxs/compare/chronicle-queue.md`. Chronicle FIX is the
  commercial layer.

#### Sources

- https://chronicle.software/fix-engine/

---

### STAC Research (third-party, not a product)

#### What it is

Securities Technology Analysis Center. Independent benchmark
authority for the financial technology industry. Membership
council of 350+ financial institutions and 50+ vendors.
Methodology development for **STAC-M2** (messaging),
**STAC-N1** (network), **STAC-M3** (tick analytics), and
**STAC-A2** (analytics), among others.

STAC's role in this document: they are the **publication carve-out**
to the DeWitt clauses common throughout commercial messaging
EULAs. Vendors who want competitive proof publish through STAC.

#### How it works

- Vendor commissions STAC to run STAC-M2 on their stack.
- STAC controls methodology, hardware, and runs.
- Vendor reviews and signs off.
- Audited report is published on `stacresearch.com` — citable by
  third parties.
- Unaudited submissions go into the **STAC Vault**, a confidential
  repository accessible only to Council members.

The STAC-M2 test harness software is itself members-only; you
cannot run a STAC-M2 benchmark yourself unless you're a council
member. You can read the published reports for free.

#### What we can do

- **Cite STAC-audited reports** in our `compare/` docs and any
  internal documents — that's the entire point of the STAC model.
- **Note that comparable RSX measurement does not have a STAC
  audit**, so any cross-citation is methodology-asymmetric.
- **Do not pursue STAC membership** for this project — not
  cost-effective.

#### Sources

- https://stacresearch.com/m2
- https://stacresearch.com/benchmarks/
- https://stacresearch.com/news/2009/11/19/ibm-releases-first-stac-benchmarks-messaging-system
- https://stacresearch.com/news/2012/02/23/juniper-networks-releases-stac-m2-benchmarks-using-qfabric-system

---

### Endace (out of scope, included for disambiguation)

#### What it is

Endace makes **packet capture appliances** (EndaceProbe). They are
nanosecond-resolution capture-and-replay devices, not messaging
middleware. Their hardware sits passively on a tap and records
every packet with hardware timestamps for forensic analysis.

Mentioned here because a casual reader might confuse "low-latency
financial network appliance" with messaging — they are not. Endace
captures *other people's* messaging traffic for analysis.

#### What we'd use them for

If we ever needed wire-level latency measurement of a real RSX
deployment, an EndaceProbe in the rack would give us
nanosecond-accurate one-way and round-trip distributions outside
the application stack. That's a deployment-time tool, not a
benchmark comparison.

#### Sources

- https://www.endace.com/

---

## Open questions

Things attempted in this research that could not be verified:

1. **Exact benchmark-publication language in the Solace, Informatica
   UM, Confinity, Real Logic / Aeron Premium, and Chronicle Software
   EULAs.** All five are either not publicly hosted in a parsable
   format, or the relevant clause was not located in the available
   text. Best assumption: standard commercial-software
   confidentiality clauses apply, which functionally restrict
   third-party benchmark publication. <!-- unverified -->

2. **Current list pricing for any of these products.** Every vendor
   uses contact-sales. Industry-report price points have been
   included with explicit "unverified" caveats; treat as
   order-of-magnitude only.

3. **Whether OnixS, Chronicle, or Solace has submitted to a STAC-M2
   audit not visible on the public STAC site (i.e. STAC Vault
   confidential submissions).** No way to verify without council
   membership.

4. **TIBCO Rendezvous's TRDP wire-format details after the
   PGM-variant removal in 8.6.x.** TIBCO documents the architecture
   but not the bytes. Customers who need wire-level analysis sign
   NDAs.

5. **Whether Aeron Premium's DPDK transport changes wire bytes
   relative to OSS Aeron's BSD-sockets transport.** Real Logic
   marketing implies "same wire, different I/O" but this is not
   stated explicitly in the docs we could reach. Best assumption:
   identical.

## Re-validation cadence

Re-validate this document when:

- A major commercial messaging product changes hands (e.g. another
  IBM-LLM-style divestiture).
- Aeron Premium publishes a public price list (would be a notable
  market signal).
- Databricks-style "DeWitt Embrace" clauses spread to messaging
  vendors. As of 2026-05-24, none of the messaging vendors profiled
  here have adopted that posture.
- STAC publishes a new STAC-M2 audit for any vendor currently in
  the "unconfirmed" column above.
- A new commercial messaging entrant appears with NAK + DPDK +
  open spec — the design space we'd genuinely want to compare CMP
  against.

The underlying business model — closed-source commercial messaging
sold under contact-sales EULAs with benchmark restrictions — has
been stable since the 1990s. Don't expect this to move quickly.
