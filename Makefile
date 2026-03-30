.PHONY: itests coverage
itests:
	@if ! command -v greentic-component >/dev/null 2>&1; then \
		echo "Skipping component integration tests: greentic-component not found on PATH"; \
	else \
		cargo test --test component_cli -- --nocapture; \
	fi

coverage:
	@bash ci/coverage.sh
