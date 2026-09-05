# Fixed /usr helper path is part of the privilege boundary. Do not relocate it independently.
CARGO ?= cargo
PROFILE ?= release
DESTDIR ?=
BUILD_DIR = target/$(PROFILE)
.PHONY: all test check install stage deb package-check
all:
	$(CARGO) build --workspace --profile $(PROFILE) --locked
test:
	$(CARGO) test --workspace --locked
check:
	$(CARGO) fmt --all --check
	$(CARGO) clippy --workspace --all-targets --locked -- -D warnings
install:
	install -Dm644 docs/VALIDATION.md $(DESTDIR)/usr/share/doc/cleanly/docs/VALIDATION.md
	install -Dm644 assets/cleanly-preview.png $(DESTDIR)/usr/share/doc/cleanly/assets/cleanly-preview.png
	install -Dm644 LICENSE $(DESTDIR)/usr/share/doc/cleanly/copyright
	install -Dm644 README.md $(DESTDIR)/usr/share/doc/cleanly/README.md
	install -Dm644 SECURITY.md $(DESTDIR)/usr/share/doc/cleanly/SECURITY.md
	install -Dm644 docs/SECURITY-REVIEW.md $(DESTDIR)/usr/share/doc/cleanly/docs/SECURITY-REVIEW.md
	install -Dm755 $(BUILD_DIR)/cleanly $(DESTDIR)/usr/bin/cleanly
	install -Dm755 $(BUILD_DIR)/cleanly-inspect $(DESTDIR)/usr/bin/cleanly-inspect
	install -Dm755 $(BUILD_DIR)/cleanly-helper $(DESTDIR)/usr/libexec/cleanly-helper
	install -Dm644 data/io.github.cleanly.Cleanly.desktop $(DESTDIR)/usr/share/applications/io.github.cleanly.Cleanly.desktop
	install -Dm644 data/io.github.cleanly.Cleanly.metainfo.xml $(DESTDIR)/usr/share/metainfo/io.github.cleanly.Cleanly.metainfo.xml
	install -Dm644 data/io.github.cleanly.Cleanly.policy $(DESTDIR)/usr/share/polkit-1/actions/io.github.cleanly.Cleanly.policy
	install -Dm644 data/icons/hicolor/scalable/apps/io.github.cleanly.Cleanly.svg $(DESTDIR)/usr/share/icons/hicolor/scalable/apps/io.github.cleanly.Cleanly.svg
stage:
	$(MAKE) install DESTDIR=$(CURDIR)/dist/stage
# Packaging rebuilds from scratch with privacy-safe path remapping.
deb:
	./scripts/package-deb.sh $(PROFILE)
package-check:
	python3 tests/package-smoke.py
