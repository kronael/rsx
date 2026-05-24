# Scenarios

Pre-built trading scenarios for testing the RSX playground.

## Built-in Scenarios

### Basic Trade

Places a buy and matching sell order to generate a fill.
Useful for verifying the matching engine and WAL recording.

### Market Maker

Starts the market maker bot which quotes bid/ask around mark price.
Verifies funding and position accumulation.

### Stress Load

Submits hundreds of orders per second across multiple symbols.
Checks for latency regression and WAL backpressure.

### Liquidation

Opens a large position then moves mark price to trigger liquidation.
Verifies the liquidator fires and margin is settled.

## Running Scenarios

From the Control page, select a scenario from the dropdown and click Run.
Scenario output appears in the Stress tab.

## Writing Scenarios

Scenarios are Python functions in `market_maker.py` that submit orders
via the REST API. See existing examples for the pattern.
