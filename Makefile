.PHONY: all build release install uninstall test clean

all: build

build:
	cargo build
	codesign -s - --entitlements entitlements.plist --force target/debug/norb

release:
	cargo build --release
	codesign -s - --entitlements entitlements.plist --force target/release/norb

install:
	./install.sh

uninstall:
	rm -f $(HOME)/.cargo/bin/norb $(HOME)/.cargo/bin/norbctl
	@echo "✓ Uninstalled norb and norbctl from $(HOME)/.cargo/bin"

test:
	cargo test

clean:
	cargo clean
