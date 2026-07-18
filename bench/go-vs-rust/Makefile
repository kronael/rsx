.PHONY: build run quick clean fmt lint

build:
	mkdir -p bin
	cd go && go build -o ../bin/ ./...
	cd rust && cargo build --release --quiet

run: ## full suite: warm-up + 5 reps per case
	./run.py

quick: ## 1 rep, no warm-up — smoke only, numbers not citable
	./run.py --quick

fmt:
	cd go && gofmt -w .
	cd rust && cargo fmt

lint:
	cd go && go vet ./...
	cd rust && cargo clippy --release --all-targets -- -D warnings

clean:
	rm -rf bin rust/target
