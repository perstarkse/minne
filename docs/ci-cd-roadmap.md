# CI/CD Roadmap: Nix-First Release Builds

This document tracks the migration from cargo-dist raw `cargo build --release` on bare GitHub runners to Nix-built release artifacts for all platforms. The goal is a single build system (the flake) shared by CI, Docker, and release binaries.

**Status:** Phase 3–4 complete locally — Nix builds all release targets including Windows cross (`nix build .#minne-release-windows` verified on x86_64-linux). cargo-dist removed from workflow and devenv. GHA tag-push validation pending.

**Decision (2026-06-23):** Drop `x86_64-apple-darwin` (Intel macOS). Ship `aarch64-apple-darwin` only; Intel Mac users can run via Rosetta 2.

---

## Executive Summary

Nix is now the sole compiler for all release binaries. Per-platform `minne-release` flake outputs produce archives compatible with GitHub Releases layout (binaries + `lib/libonnxruntime.*` + docs). The release workflow uses matrix jobs running `nix build` with `cache-nix-action` on every job. cargo-dist has been removed; releases use `gh release create` with CHANGELOG-driven notes.

This fixes the mozangle/clang failure at the root: the flake already wires `libclang`, `bindgenHook`, `llvm`, `python3`, `fontconfig`, and `MOZJS_ARCHIVE` — cargo-dist on bare Ubuntu cannot see any of that without duplicating it in apt/workflow steps.

---

## Current State

### What works

- [x] CI (`nix flake check`) — format, clippy, tests, ort-version gate via Crane `buildDepsOnly`
- [x] Release Docker job — `nix build .#dockerImage`, push to GHCR with dynamic tag from `docker load`
- [x] Release plan job — `nix flake check`, ORT version from flake (no cargo-dist)
- [x] Harmonized native deps in `flake.nix` for CI/Docker (openssl, libglvnd, onnxruntime, fontconfig, bindgen, mozjs)

### What is broken or painful

- [x] ~~cargo-dist Linux build fails without apt/mozjs workarounds~~ — resolved: Nix builds all platforms
- [x] ~~Two build systems: Nix for CI/Docker, cargo + apt/homebrew for dist binaries~~ — resolved: Nix-only release
- [x] ~~Four independent release compiles with no shared Nix store across jobs~~ — resolved: `cache-nix-action` on all release jobs
- [x] ~~`[dist.dependencies.apt]` duplicates flake.nix logic~~ — resolved: `dist-workspace.toml` deleted
- [ ] Release compile time on GHA not yet measured post-migration (expected ~10–25 min warm vs ~50–110 min cold)
- [ ] GHA tag-push validation pending for macOS and Windows archives

---

## Target Architecture

| Layer | Owner | Notes |
|-------|-------|-------|
| Compile binaries | **Nix** (`minne-pkg` / cross derivations) | Crane + `commonArgs`, per-platform mozjs |
| Bundle ORT + runtime libs | **Nix** (`minne-release`) | Match `include = ["lib"]` layout |
| Create archives | **Nix** or thin shell | `.tar.xz` (Unix), `.zip` (Windows) |
| Publish GitHub Release | **`gh release create`** | CHANGELOG body |
| Docker image | **Nix** (unchanged) | Shares `minne-pkg` derivation with Linux release |
| cargo-dist | **Removed** | Replaced by Nix jobs + `gh release` |

### Release targets (end state)

| Target | Builder | Nix output |
|--------|---------|------------|
| `x86_64-unknown-linux-gnu` | `ubuntu-22.04` native | `.#minne-release` |
| `aarch64-apple-darwin` | `macos-latest` native | `.#minne-release` |
| ~~`x86_64-apple-darwin`~~ | **Dropped** | — |
| `x86_64-pc-windows-msvc` | `ubuntu-22.04` cross | `.#minne-release-windows` |

---

## Per-Platform Build Matrix

| Target | Feasibility | Nix command | Artifact layout | Blockers |
|--------|-------------|-------------|-----------------|----------|
| `x86_64-unknown-linux-gnu` | Ready with modest flake changes | `nix build .#minne-release --system x86_64-linux` | `main-{ver}-x86_64-unknown-linux-gnu.tar.xz` → `main/`, `server/`, `worker/`, `lib/libonnxruntime.so`, README, LICENSE, CHANGELOG | glibc 2.40 (nixpkgs-unstable) vs Ubuntu 22.04 glibc 2.35; portable runtime bundling needed |
| `aarch64-apple-darwin` | Feasible | `nix build .#minne-release --system aarch64-darwin` | `main-{ver}-aarch64-apple-darwin.tar.xz` + `lib/libonnxruntime.dylib` | Per-system mozjs URL; Darwin `postInstall` assumes Linux today |
| `x86_64-pc-windows-msvc` | Feasible with new cross flake | `nix build .#minne-release-windows` (x86_64-linux host) | `main-{ver}-x86_64-pc-windows-msvc.zip` + `lib/onnxruntime.dll` | Crane + cargo-xwin cross setup; no native Nix-on-Windows for v1 |

### mozjs prebuilt availability (mozjs-sys-v140.10.1-0)

Confirmed for all release triples:

- `libmozjs-x86_64-unknown-linux-gnu.tar.gz`
- `libmozjs-aarch64-apple-darwin.tar.gz`
- `libmozjs-x86_64-pc-windows-msvc.tar.gz`

---

## Caching Strategy

| Layer | Invalidated by | Shared across |
|-------|----------------|---------------|
| Nix store (system deps) | `flake.lock`, `*.nix`, `Cargo.lock` | plan, CI, Docker, all release jobs (per OS) |
| `cargoArtifacts` (`buildDepsOnly`) | `Cargo.lock` dep changes only | minne-pkg, clippy, test, dockerImage, release |
| `minne-pkg` (source) | Application source changes | dockerImage, release |
| cargo-dist `target/` | Version bumps (weak) | Removed — Nix store replaces it |

### Expected release times

| Scenario | Current (cargo-dist) | After migration (Nix) |
|----------|---------------------|-------------------------|
| Cold release (version bump, no cache) | ~50–110 min × 4 jobs | ~45–90 min × 3 jobs (no Intel Mac) |
| Warm release (source-only, cache hit) | Still ~full rebuild | ~10–25 min incremental per OS |
| No-op re-release | Full rebuild | ~2–5 min if derivations unchanged |
| Docker job (cached) | ~5–15 min | Unchanged; shares `minne-pkg` with Linux release |

`buildDepsOnly` survives version bumps (version is in flake `minneVersion`, not `Cargo.lock`) — major win over cargo-dist.

---

## Implementation Phases

### Phase 1 — Linux via Nix (highest pain, highest value)

- [x] Add per-system `mozjsArchive` helper with hash map (at minimum fix structure for all platforms)
- [x] Add `nix/minne-release.nix` — bundle ORT + portable runtime libs + docs into archive
- [x] Linux portable runtime lib bundling + `patchelf --set-rpath '$ORIGIN/lib'`
- [x] Replace Linux `build-local-artifacts` steps with `nix build .#minne-release`
- [x] Add `cache-nix-action` to Linux release build job
- [x] Validate glibc portability (test binary on Ubuntu 22.04)
- [x] Remove Linux apt/mozjs/ORT curl workarounds from `.github/workflows/release.yml`
- [x] Archive naming matches prior releases (`main-{triple}.tar.xz`, no version in filename)

### Phase 2 — macOS aarch64

- [x] Platform-conditional `postInstall` in `flake.nix` (Darwin vs Linux wrapping)
- [x] Add `nix/minne-release-darwin.nix` — ORT + runtime dylibs + docs archive
- [x] macOS `build-local-artifacts` uses `nix build .#minne-release` on `macos-latest`
- [x] `cache-nix-action` on macOS release build job
- [x] Drop `x86_64-apple-darwin` target (was in `dist-workspace.toml`, now deleted)
- [ ] Test archive on clean macOS VM / GHA release run
- [x] Update `docs/installation.md` to note aarch64-only macOS binary (Rosetta 2 for Intel Macs)

### Phase 3 — Windows cross from Linux

- [x] Add `minne-release-windows` cross derivation (Crane + cargo-xwin)
- [x] Add `nix/clang-cl-msvc-link-wrapper.sh` for mozangle DLL links under clang-cl
- [x] Windows GHA job on `ubuntu-22.04` (cross-build, not Nix-on-Windows)
- [x] Bundle `onnxruntime.dll` in release zip (match cargo-dist flat layout)
- [x] Fenix `rust-std` for `x86_64-pc-windows-msvc` via `fenix.combine`
- [x] Local cross-build verified: `nix build .#minne-release-windows` on x86_64-linux
- [ ] Test archive on Windows VM / GHA release run

### Phase 4 — Cleanup

- [x] Remove cargo-dist compile steps from release workflow
- [x] Delete `dist-workspace.toml`
- [x] Simplify CI to `nix flake check` only (drop `cargo-dist plan`)
- [x] Replace `host`/cargo-dist with `gh release create` + CHANGELOG
- [x] Remove `pkgs.cargo-dist` from `devenv.nix`
- [x] Update `AGENTS.md` release checklist
- [ ] Update README release badges/docs if workflow structure changes

---

## Flake Changes (outline)

New/modified outputs:

```nix
# Per-system mozjs (replace hardcoded Linux x86_64)
mozjsTarget = { "x86_64-linux" = "x86_64-unknown-linux-gnu"; ... }.${system};
mozjsArchive = pkgs.fetchurl { url = ".../libmozjs-${mozjsTarget}.tar.gz"; hash = mozjsHashes.${system}; };

# Platform-conditional postInstall (Linux LD_LIBRARY_PATH vs Darwin)

# NEW: release archive derivation
packages.minne-release = callPackage ./nix/minne-release.nix { inherit minne-pkg minneVersion ortVersion; };

# NEW: Windows cross (x86_64-linux host only)
packages.minne-release-windows = ...;
```

New file: `nix/minne-release.nix` — copies stripped binaries, stages `lib/libonnxruntime.{so,dylib}`, optional runtime `.so` copies, includes README/LICENSE/CHANGELOG, builds `.tar.xz` / `.zip`.

Optional: `devShells.dist` for local release-build debugging.

---

## Workflow Changes (outline)

Target `release.yml` structure:

```
plan:
  - nix flake check, nix eval ortVersion
  - output tag from github.ref (no hardcoded versions)

build-nix-artifacts:          # replaces build-local-artifacts
  matrix: linux | macos-aarch64 | windows-cross
  - determinate-nix + cache-nix-action on ALL jobs
  - nix build .#${attr} --system ${system} -L
  - upload: main-*-{triple}.tar.xz / .zip

build_and_push_docker_image:  # unchanged

release:                      # replaces build-global-artifacts + host
  - download artifacts
  - gh release create with CHANGELOG body
```

Artifact naming: match current convention for backwards compatibility — `main-{version}-{triple}.tar.xz` (Unix) / `.zip` (Windows).

---

## cargo-dist Fate

**Status:** Removed (Option B implemented in Phase 4).

| Option | Verdict |
|--------|---------|
| A) Nix builds → cargo-dist packages only | No clean skip-compile mode; high friction |
| **B) Replace with custom Nix jobs + `gh release`** | **Implemented** |
| C) `build-local-artifacts = false` + custom jobs | Experimental; superseded by Option B |

---

## Task Checklist (with complexity)

| # | Task | Size | Phase | Done |
|---|------|------|-------|------|
| 1 | `mozjsArchive` per `system` with hash map | S | 1 | [x] |
| 2 | Platform-conditional `postInstall` in flake | S | 1–2 | [x] |
| 3 | `nix/minne-release.nix` archive bundler | M | 1 | [x] |
| 4 | Linux portable runtime lib bundling + patchelf | M | 1 | [x] |
| 5 | Replace Linux `build-local-artifacts` with Nix job | S | 1 | [x] |
| 6 | Add `cache-nix-action` to all release build jobs | S | 1–3 | [x] |
| 7 | glibc portability test + fix | M | 1 | [x] |
| 8 | Darwin release bundle + macOS GHA job | M | 2 | [x] |
| 9 | Drop `x86_64-apple-darwin` from targets | S | 2 | [x] |
| 10 | Windows cross flake (`minne-release-windows`) | L | 3 | [x] |
| 11 | Windows GHA job | S | 3 | [x] |
| 12 | Replace `host`/cargo-dist with `gh release` | S | 4 | [x] |
| 13 | Remove apt deps, ORT curl, cargo-dist install | S | 4 | [x] |
| 14 | Update docs/AGENTS release checklist | S | 4 | [x] |

S = hours–1 day, M = 2–4 days, L = 1–2 weeks

---

## Risks & Blockers

| Risk | Severity | Mitigation | Resolved |
|------|----------|------------|----------|
| glibc compatibility (nixpkgs 2.40 vs Ubuntu 22.04 2.35) | High | Bundle runtime libs in `lib/` + `LD_LIBRARY_PATH` wrappers; bundled glibc interpreter | [x] |
| mozjs per-platform hashes drift on `Cargo.lock` bump | Medium | Centralize in `mozjsHashes` attrset; document bump procedure | [ ] |
| Darwin `postInstall` assumes Linux (`LD_LIBRARY_PATH`, `libglvnd`) | Medium | Platform-conditional wrapping in flake | [x] |
| Windows cross complexity (Crane + cargo-xwin) | Medium–High | cargo-xwin env + clang-cl wrapper for mozangle; Dbghelp.lib case symlink | [x] |
| Nix on macOS GHA speed | Medium | cache-nix-action; larger runner if needed | [ ] |
| Codesigning / notarization (macOS) | Low | Not required for CLI today; document `xattr` workaround; revisit if needed | [ ] |
| musl target (`x86_64-unknown-linux-musl`) | N/A | mozjs/servo stack is glibc-oriented; stay on `*-linux-gnu` unless explicitly requested | [ ] |
| ORT version drift | Low | Existing `ortVersion` gate in flake + devenv | [x] |

---

## Open Questions

1. **glibc portability strategy** — Bundle runtime libs in `lib/` (preferred for portability) vs pin `nixpkgs` to an older release channel for release builds vs document minimum distro? Need a test matrix: Ubuntu 22.04, Debian 12, Fedora current.

2. **Archive format** — Confirmed: `.tar.xz` (Unix), `.zip` (Windows); naming `main-{triple}.*` (no version in filename).

3. **Binary scope** — Release all three binaries (`main`, `server`, `worker`) in one archive per platform (unchanged from prior cargo-dist behavior).

4. **PR artifact builds** — Not implemented; cargo-dist `pr-run-mode` was disabled. Revisit if PR smoke-test artifacts are wanted.

5. **Cachix** — Deferred; `cache-nix-action` on all release jobs is sufficient for now.

6. **Windows cross approach** — Resolved: Crane + offline xwin MSVC cache + fenix `rust-std` + clang-cl/lld-link shims (`nix build .#minne-release-windows` verified locally).

7. **Version source of truth** — Release workflow reads version from flake (`minneVersion`).

8. **cargo-dist removal timing** — Resolved: removed in Phase 4.

9. **Intel Mac deprecation communication** — Done: `docs/installation.md` notes aarch64-only + Rosetta 2.

---

## Success Criteria

After implementation:

- [x] Release workflow no longer runs raw `cargo build --release` on bare GitHub runners
- [x] Native deps (clang, mozjs, onnxruntime, etc.) come from flake/Nix, not apt
- [x] Linux, macOS (aarch64), and Windows release binaries are produced via Nix
- [x] Docker and release binaries share maximum Nix store cache (`cache-nix-action` on all jobs)
- [x] No hardcoded version strings in `release.yml`
- [ ] Warm release compile time materially improved (~10–25 min/platform vs ~50–110 min today) — pending GHA measurement
- [ ] macOS and Windows archives validated on clean VM / GHA tag-push release run

---

## References

- [Crane cross-windows example](https://crane.dev/examples/cross-windows.html)
- [Crane discussion: MSVC / cargo-xwin](https://github.com/ipetkov/crane/discussions/555)
- [cargo-dist CI customization](https://axodotdev.github.io/cargo-dist/book/ci/customizing.html)
- [servo/mozjs releases](https://github.com/servo/mozjs/releases)
- Project files: `flake.nix`, `.github/workflows/release.yml`, `devenv.nix`, `nix/minne-release*.nix`
