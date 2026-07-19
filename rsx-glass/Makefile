# rsx-glass — the desktop/GPU renderer for the rsx-term visual language.
#
# The whole test suite is HEADLESS: no display, no GPU, no live cluster. The
# integration suite runs the full live path (exchange wire -> connector -> model
# -> Grid -> soft) against an in-process mock exchange (mockx); the contract
# slice pins the mock's frames against the real producer's committed byte
# vectors. `make test` is the default gate.

.PHONY: all test test-race build vet fmt fmt-check contract-live clean

all: fmt-check vet build test ## fmt-check + vet + build + headless test suite

test: ## the whole headless suite (mock exchange, no display, no cluster)
	go test ./...

test-race: ## the suite under the race detector
	go test -race ./...

build: ## compile every package (incl. the build-tagged live contract test)
	go build ./...
	go vet -tags contract ./live/

vet: ## go vet
	go vet ./...

fmt: ## rewrite with gofmt
	gofmt -w .

fmt-check: ## fail if any file is not gofmt-clean
	@out=$$(gofmt -l .); if [ -n "$$out" ]; then echo "gofmt needed:"; echo "$$out"; exit 1; fi

# The live half of the mock-stays-honest contract: connect to a REAL feed and
# confirm the mock reproduces its frames. Skips cleanly without RSX_MD_URL.
contract-live: ## verify the mock against a running marketdata feed (needs RSX_MD_URL)
	go test -tags contract ./live/ -run Contract -v

clean: ## remove build/test caches for this module
	go clean -testcache ./...
