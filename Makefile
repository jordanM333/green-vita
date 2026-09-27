.PHONY: vpk eboot upload-vpk update-run-vita run-vita

RUSTFLAGS ?= -C target-feature=-neon
CARGO_VITA ?= cargo vita
VPK := target/armv7-sony-vita-newlibeabihf/release/green-vita.vpk
TEST_VPK := target/armv7-sony-vita-newlibeabihf/release/GreenVita-540p-2000k-Test.vpk
VITA_UPLOAD_DIR ?= ux0:/data/

vpk: sdk-check
	RUSTFLAGS="$(RUSTFLAGS)" $(CARGO_VITA) build vpk --release --locked

eboot: sdk-check
	RUSTFLAGS="$(RUSTFLAGS)" $(CARGO_VITA) build eboot --release --locked

upload-vpk: vpk
ifndef VITA_IP
	$(error Usage: make upload-vpk VITA_IP=192.168.0.103)
endif
	cp $(VPK) $(TEST_VPK)
	$(CARGO_VITA) upload --vita-ip $(VITA_IP) --source $(TEST_VPK) --destination $(VITA_UPLOAD_DIR)

update-run-vita: sdk-check
ifndef VITA_IP
	$(error Usage: make update-run-vita VITA_IP=192.168.0.103)
endif
	RUSTFLAGS="$(RUSTFLAGS)" $(CARGO_VITA) build eboot --update --run --vita-ip $(VITA_IP) -- --release --locked

run-vita: update-run-vita

.PHONY: sdk-check native-check native-clippy host-tests
sdk-check:
	python3 tools/check_sdk.py

native-check: sdk-check
	cargo check --locked -Zbuild-std=std,panic_abort --all-targets --all-features

native-clippy: sdk-check
	cargo clippy --locked -Zbuild-std=std,panic_abort --all-targets --all-features -- -D warnings

host-tests:
	python3 tools/run_host_audit.py --output verification/host --clippy
