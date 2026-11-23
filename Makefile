# SPDX-FileCopyrightText: 2026 Marc Seibert
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# The definition of done (AGENTS.md).
.PHONY: check check-public secret-scan fmt clippy test api-standalone license-check deny reuse-lint web-check emacs-check install install-handler install-replica install-front android

PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
APPDIR ?= $(PREFIX)/share/applications

check: check-public

# What the public CI runs (.github/workflows/ci.yml), and what a
# contribution needs. A local.mk beside this file, if there is one, can add
# checks of its own to `check`.
check-public: fmt clippy test api-standalone license-check deny reuse-lint web-check emacs-check

fmt:
	cargo fmt --all --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

# samu-api is published on its own, on crates.io as samudo-api
# (ADR 0003 §1, ADR 0137 §7). This build makes sure it carries nothing from this
# workspace's other crates and still packages: `cargo package` verifies the
# crate builds from exactly the files that would be uploaded.
api-standalone:
	cargo package -p samudo-api --allow-dirty

# The crates that may also be used under MIT must not depend on one that
# is only AGPL (ADR 0117).
license-check:
	scripts/license-check.sh

# Known vulnerabilities (RustSec), the licenses of all dependencies and
# where crates come from, as deny.toml says. Fetches the advisory database
# on each run, so a new advisory fails a tree that passed yesterday. Needs
# cargo-deny (cargo install --locked cargo-deny).
deny:
	@command -v cargo-deny >/dev/null 2>&1 || { echo "deny: cargo-deny is missing (cargo install --locked cargo-deny)" >&2; exit 1; }
	cargo deny check

# Every file carries its copyright and license: an SPDX header in source
# files, an entry in REUSE.toml for data, images and third-party code
# (https://reuse.software). Needs the reuse tool (Debian: apt install reuse).
reuse-lint:
	@command -v reuse >/dev/null 2>&1 || { echo "reuse-lint: reuse is missing (apt install reuse, or pipx install reuse)" >&2; exit 1; }
	reuse lint

# A secret scan of exactly what is committed (ADR 0108 §7), with the
# exceptions in .gitleaks.toml. Needs gitleaks; not part of check, since it
# is an outside binary.
secret-scan:
	@command -v gitleaks >/dev/null 2>&1 || { echo "secret-scan: gitleaks is missing" >&2; exit 1; }
	@tree=$$(mktemp -d) && trap 'rm -rf "$$tree"' EXIT && \
		git archive HEAD | tar -x -C "$$tree" && \
		gitleaks dir "$$tree" --config .gitleaks.toml --no-banner --redact

# The web's search offers against the cases samu-core meets as well
# (ADR 0103 §2), and the English tables against the keys (ADR 0105 §5).
# Needs node (Debian: nodejs), no packages; without it the
# check fails rather than skip the probe.
web-check:
	@command -v node >/dev/null 2>&1 || { echo "web-check: node is missing: apt install nodejs (ADR 0103 §2)" >&2; exit 1; }
	node web/tests/offers.mjs
	node web/tests/author.mjs
	node web/tests/transcript.mjs
	node web/tests/i18n.mjs
	node web/tests/lang.mjs
	node web/tests/table.mjs
	node web/tests/clicks.mjs
	node web/tests/blocks.mjs
	node web/tests/blocklines.mjs
	node web/tests/close.mjs
	node web/tests/together.mjs
	node web/tests/checklist.mjs
	node web/tests/text.mjs
	node web/tests/groups.mjs
	node web/tests/clocks.mjs
	node web/tests/rowfields.mjs
	node web/tests/arrange.mjs
	node web/tests/tagfields.mjs
	node web/tests/jump.mjs
	node web/tests/message.mjs
	node web/tests/route.mjs

# samu.el: compiled with warnings as errors, then its ERT tests, both
# without a CLI or a server (ADR 0082 §8). Skipped, and
# said so, where there is no emacs.
EMACS ?= emacs
emacs-check:
	@if command -v $(EMACS) >/dev/null 2>&1; then \
		$(EMACS) -Q --batch -L emacs \
			--eval '(setq byte-compile-error-on-warn t)' \
			--eval '(setq byte-compile-dest-file-function (lambda (_) (make-temp-file "samu-" nil ".elc")))' \
			-f batch-byte-compile emacs/samu.el && \
		$(EMACS) -Q --batch -L emacs -l emacs/samu.el -l emacs/test-samu.el \
			-f ert-run-tests-batch-and-exit; \
	else \
		echo "emacs-check: no $(EMACS), skipped" >&2; \
	fi

# The command line on this workstation. Written beside the target and moved
# into place, never over it: truncating a running binary hands it a SIGBUS
# (as in catastr).
install: install-handler
	cargo build --release --locked --bin samu
	install -d $(BINDIR)
	install -m 0755 $$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)/release/samu $(BINDIR)/samu.new
	mv $(BINDIR)/samu.new $(BINDIR)/samu
	@$(BINDIR)/samu --version

# The handler for samu-open:<uuid>/<key>, the address the web hands a
# click on a property to (ADR 0054 §6), and for samu:<uuid>, which opens
# the object in the web UI (ADR 0075 §2). The browser asks before it
# passes one on; the address carries no value, samu reads it itself. A
# target of its own for a computer that gets the binary without building.
install-handler:
	install -d $(APPDIR)
	printf '%s\n' '[Desktop Entry]' 'Type=Application' 'Name=samudo open' \
		'Comment=Opens a samudo address: a task, or a property of one' \
		'Exec=$(BINDIR)/samu open-uri --notify %u' 'NoDisplay=true' 'Terminal=false' \
		'MimeType=x-scheme-handler/samu-open;x-scheme-handler/samu;' > $(APPDIR)/samu-open.desktop
	-command -v update-desktop-database >/dev/null && update-desktop-database $(APPDIR)
	-command -v xdg-mime >/dev/null && xdg-mime default samu-open.desktop x-scheme-handler/samu-open x-scheme-handler/samu

# The profiles of this computer (ADR 0086): the server binary and the
# user unit samu@.service, one instance per profile. Enabling one is a
# step of its own (deploy/samu@.service).
UNITDIR ?= $(HOME)/.config/systemd/user
install-replica:
	cargo build --release --locked --bin samu-server
	install -d $(BINDIR) $(UNITDIR)
	install -m 0755 $$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)/release/samu-server $(BINDIR)/samu-server.new
	mv $(BINDIR)/samu-server.new $(BINDIR)/samu-server
	install -m 0755 deploy/samu-export-profile.sh $(BINDIR)/samu-export-profile
	install -m 0644 deploy/samu@.service deploy/samu-export@.service deploy/samu-export@.timer $(UNITDIR)/
	-systemctl --user daemon-reload
	@$(BINDIR)/samu-server --version
	@# A profile that keeps running keeps the old binary, and with it the old
	@# schema: migrations run when the server starts, so new routes answer 404
	@# and the export stops at "schema N, this build at M" (16.11.2026).
	@for u in $$(systemctl --user list-units 'samu@*.service' --state=active --plain --no-legend 2>/dev/null | cut -d' ' -f1); do \
		echo "$$u still runs the old samu-server: systemctl --user restart $$u"; done
	@if systemctl is-active --quiet samu-front 2>/dev/null; then \
		echo "samu-front still runs the old samu-server: sudo systemctl restart samu-front"; fi

# The front on port 80 (ADR 0087 §4): a system unit that runs as this user
# and may take only that port. Needs sudo, and samu-server from
# install-replica.
install-front:
	test -x $(BINDIR)/samu-server
	sed -e 's|@USER@|$(USER)|g' -e 's|@BINDIR@|$(BINDIR)|g' deploy/samu-front.service \
		| sudo install -m 0644 /dev/stdin /etc/systemd/system/samu-front.service
	sudo systemctl daemon-reload
	sudo systemctl enable samu-front
	sudo systemctl restart samu-front
	@grep -q '^\[front\]' $(HOME)/.config/samu/config.toml 2>/dev/null || \
		echo "add [front] to ~/.config/samu/config.toml, so the CLI opens http://<profile>.samu.localhost"

# The Android app (ADR 0028), outside `check`: it needs the SDK, the NDK and
# cargo-ndk (android/README.md). Gradle runs android/rust.sh first.
android:
	cd android && ./gradlew assembleDebug

-include local.mk
