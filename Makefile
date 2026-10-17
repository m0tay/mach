.PHONY: all build release install uninstall test clean

all: build

build:
	cargo build
	codesign -s - --entitlements entitlements.plist --force target/debug/mach

release:
	cargo build --release
	codesign -s - --entitlements entitlements.plist --force target/release/mach

install:
	./install.sh

uninstall:
	rm -f $(HOME)/.cargo/bin/mach $(HOME)/.cargo/bin/machctl
	@echo "✓ Uninstalled mach and machctl from $(HOME)/.cargo/bin"

test:
	cargo test

clean:
	cargo clean
