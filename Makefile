.PHONY: build check test fmt lint audit clean deploy-spark docker-build

# Target platform for Spark (DGX ARM64)
TARGET_ARCH := aarch64-unknown-linux-gnu
SPARK_HOST := dgx-spark
SPARK_BIN_DIR := /usr/local/bin
SPARK_COMPOSE_DIR := /opt/dgx-llm

# === Development ===

build:
	cargo build --release

build-cross:
	cross build --release --target $(TARGET_ARCH)

check:
	cargo fmt --all -- --check
	cargo clippy --all -- -D warnings
	cargo nextest run --all

fmt:
	cargo fmt --all

lint:
	cargo clippy --all -- -D warnings

test:
	cargo nextest run --all

audit:
	cargo audit
	cargo deny check

clean:
	cargo clean

# === Docker ===

docker-build:
	docker buildx build \
		--platform linux/arm64 \
		-f docker/Dockerfile.native \
		-t nexus-link-service:latest \
		--load .

docker-push: docker-build
	docker tag nexus-link-service:latest $(REGISTRY)/nexus-link-service:latest
	docker push $(REGISTRY)/nexus-link-service:latest

# === Deployment to Spark ===

deploy-cli:
	cross build --release --target $(TARGET_ARCH) -p nexus-link-cli
	rsync -avz target/$(TARGET_ARCH)/release/nexus-link $(SPARK_HOST):$(SPARK_BIN_DIR)/

deploy-agent:
	cross build --release --target $(TARGET_ARCH) -p nexus-link-agent
	rsync -avz target/$(TARGET_ARCH)/release/nexus-link-agent $(SPARK_HOST):$(SPARK_BIN_DIR)/

deploy-service:
	@echo "Service is deployed as container via docker compose on Spark"
	@echo "Build and push the image, then restart on Spark:"
	@echo "  make docker-build"
	@echo "  scp docker/Dockerfile.native $(SPARK_HOST):$(SPARK_COMPOSE_DIR)/"
	@echo "  ssh $(SPARK_HOST) 'cd $(SPARK_COMPOSE_DIR) && docker compose up -d nexus-link-service'"

deploy-all: deploy-cli deploy-agent
	@echo "CLI and agent deployed to $(SPARK_HOST)"
	@echo "Service runs as container -- use 'make deploy-service' for instructions"

# === Spark Operations ===

spark-status:
	ssh $(SPARK_HOST) "nexus-link status"

spark-logs:
	ssh $(SPARK_HOST) "journalctl -u nexus-link-agent -n 50 --no-pager"

spark-compose-status:
	ssh $(SPARK_HOST) "cd $(SPARK_COMPOSE_DIR) && docker compose ps"
