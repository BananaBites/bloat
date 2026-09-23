# bloat — local development helpers.
#
# Installing does not need this file:
#   cargo install --git https://github.com/BananaBites/bloat
#   curl -fsSL https://raw.githubusercontent.com/BananaBites/bloat/main/install.sh | sh
#
# Everything here runs against the working tree:
#   make                 build the debug binaries (bin + tests)
#   make run             interactive treemap of DIR (default: this directory)
#   make report          static report of DIR
#   make test            the test suite
#   make release         optimised binary -> target/release/bloat
#   make run DIR=/var    analyse another directory
#   make report ARGS=-a  extra flags for either run target

CARGO ?= cargo
DIR ?= .
ARGS ?=

.DEFAULT_GOAL := all

.PHONY: all build release test run report doctor install clean help

all: build ## default: build everything (debug)

build: ## build all targets (binary and tests) in debug mode
	$(CARGO) build --all-targets

release: ## build the optimised binary at target/release/bloat
	$(CARGO) build --release

test: ## run the test suite
	$(CARGO) test

run: ## interactive treemap for DIR (release build)
	$(CARGO) run --release -- $(ARGS) $(DIR)

report: ## static report for DIR, no TUI
	$(CARGO) run --release -- --report $(ARGS) $(DIR)

doctor: ## run the dev build's doctor (local vs remote commit, environment)
	$(CARGO) run --release -- doctor

install: ## install this working tree into ~/.cargo/bin
	$(CARGO) install --path . --force

clean: ## remove build output
	$(CARGO) clean

help: ## list these targets
	@echo "bloat local development targets:"
	@grep -hE '^[a-z]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN { FS = ":.*?## " } { printf "  \033[36m%-8s\033[0m %s\n", $$1, $$2 }'
	@echo
	@echo "variables: DIR=$(DIR)  ARGS='$(ARGS)'"
	@echo "examples:  make run DIR=/var   ·   make report ARGS='-a -e *.iso'"
