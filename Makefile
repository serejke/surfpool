# Docker build/run helpers for the surfpool image.
# See Dockerfile for the caching strategy (cargo-chef + BuildKit cache mounts).
#
# The default run target uses the published image at GHCR; `make build`
# produces a local image (LOCAL_IMAGE) that you can run by overriding IMAGE:
#   make build
#   IMAGE=$(LOCAL_IMAGE) make run

LOCAL_IMAGE ?= surfpool:local
IMAGE       ?= ghcr.io/serejke/surfpool:1.3.1-ghcr.1
BUILDER     ?= desktop-linux
# Force the amd64 manifest so the image runs on Apple Silicon via QEMU.
# Native linux/amd64 hosts ignore this flag.
PLATFORM    ?= linux/amd64

DOCKER_BUILD = DOCKER_BUILDKIT=1 docker buildx build \
	--builder $(BUILDER) \
	--progress=plain \
	--load \
	-t $(LOCAL_IMAGE) \
	.

.DEFAULT_GOAL := help

.PHONY: help pull build rebuild cold-build run clean clean-cache

help:
	@echo "Targets:"
	@echo "  pull         Pull $(IMAGE) (default: GHCR) for the configured platform"
	@echo "  run          Run $(IMAGE) with default ports exposed on localhost"
	@echo "  build        Build $(LOCAL_IMAGE) via BuildKit (cache mounts reused)"
	@echo "  rebuild      Re-run every RUN (--no-cache); cache mounts still reused"
	@echo "  cold-build   Prune cache mounts then build from scratch"
	@echo "  clean        Remove the $(IMAGE) image"
	@echo "  clean-cache  Prune BuildKit cache mounts for this project"
	@echo
	@echo "Overrides: IMAGE=<tag> LOCAL_IMAGE=<tag> BUILDER=<buildx-builder> PLATFORM=<linux/amd64|linux/arm64>"

pull:
	docker pull --platform $(PLATFORM) $(IMAGE)

build:
	$(DOCKER_BUILD)

rebuild:
	$(DOCKER_BUILD) --no-cache

cold-build: clean-cache build

run:
	docker run --rm -it \
		--platform $(PLATFORM) \
		-p 8899:8899 -p 8900:8900 -p 18488:18488 \
		$(IMAGE)

clean:
	-docker rmi $(IMAGE)

clean-cache:
	docker buildx prune --builder $(BUILDER) --force \
		--filter type=exec.cachemount \
		--filter "id~=^surfpool-"
