# Docker build helpers for the surfpool image.
# See Dockerfile for the caching strategy (cargo-chef + BuildKit cache mounts).

IMAGE   ?= surfpool:local
BUILDER ?= desktop-linux

DOCKER_BUILD = DOCKER_BUILDKIT=1 docker buildx build \
	--builder $(BUILDER) \
	--progress=plain \
	--load \
	-t $(IMAGE) \
	.

.DEFAULT_GOAL := help

.PHONY: help build rebuild cold-build run clean clean-cache

help:
	@echo "Targets:"
	@echo "  build        Build $(IMAGE) using BuildKit layer + cache-mount caches"
	@echo "  rebuild      Re-run every RUN (--no-cache); cache mounts still reused"
	@echo "  cold-build   Prune cache mounts then build from scratch"
	@echo "  run          Run $(IMAGE) with default ports exposed on localhost"
	@echo "  clean        Remove the $(IMAGE) image"
	@echo "  clean-cache  Prune BuildKit cache mounts for this project"
	@echo
	@echo "Overrides: IMAGE=<tag> BUILDER=<buildx-builder>"

build:
	$(DOCKER_BUILD)

rebuild:
	$(DOCKER_BUILD) --no-cache

cold-build: clean-cache build

run:
	docker run --rm -it \
		-p 8899:8899 -p 8900:8900 -p 18488:18488 \
		$(IMAGE)

clean:
	-docker rmi $(IMAGE)

clean-cache:
	docker buildx prune --builder $(BUILDER) --force \
		--filter type=exec.cachemount \
		--filter "id~=^surfpool-"
