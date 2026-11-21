# Isolated Windows authentication crash reproducer

This diagnostic program is independent of Fexu, SQLite, Tokio, HTTP, sessions and application databases. It uses only the same Argon2 0.5.3 algorithm/parameters as `fexu-auth`: Argon2id v0x13, memory 19456 KiB, time2, parallelism1. It hashes disposable constant input with a disposable salt in independent OS threads. This is a reproducer, not a production password helper or a fix.

## Reproduce

From this directory on Windows x86_64, with native Rust build prerequisites:

```powershell
cargo build --locked
.\target\debug\fexu-auth-crash-minimal.exe 24 8388608
```

Arguments are the OS thread count and each thread's stack reservation in bytes. The default is24 threads and2MiB stacks. Each thread performs two independent hashes. To compare the minimum-supported toolchain and optimized code without changing the hashing parameters:

```powershell
cargo +1.89.0 build --locked --target-dir target-msrv
.\target-msrv\debug\fexu-auth-crash-minimal.exe 24 8388608
cargo build --locked --release
.\target\release\fexu-auth-crash-minimal.exe 24 8388608
```

Do not reduce test parallelism, disable AVX2, weaken hashing parameters or change the release profile as an assumed fix. A passing variant is diagnostic evidence only.

## Observed 2026-11-21

On the investigated Windows11 x86_64 machine with24 logical CPUs:

| Binary/configuration | Observed result |
|---|---|
| Original `fexu-auth` test binary, default test thread count | Immediate `0xc0000005` on a direct repeat; CDB captured an unhandled second-chance exception and a private full dump |
| Original auth tests under CDB with8 test threads |12/12 complete runs passed:48 tests and3 explicitly ignored PostgreSQL tests each |
| Password-only / SQLite-only / reconnect-only groups under CDB |6/6 runs per group passed |
| Standalone Argon2 program, Rust1.98.1 debug,1 or8 threads,2MiB stacks |Passed the executed one-run comparisons |
| Standalone program, Rust1.98.1 debug,24 threads,2MiB and8MiB stacks |Access violation; a second-chance dump captured with8MiB stacks |
| Standalone program, Rust1.98.1 debug,24 threads,8MiB stacks |3/3 additional direct runs exited3221225477 (`0xc0000005`) |
| Standalone program, Rust1.89.0 debug,24 threads,8MiB stacks |3/3 direct runs exited3221225477 |
| Standalone program, Rust1.98.1 release,24 threads,8MiB stacks |3/3 runs passed |

The captured auth stack includes:

```text
ExceptionCode: c0000005
ExceptionAddress: 0000000000000001
core::slice::iter::...::next<u64>
argon2::block::Block::compress
argon2::...::compress_avx2
argon2::Argon2::compress
argon2::Argon2::fill_blocks
argon2::Argon2::hash_password_into_with_memory
fexu_auth::password::verify_password
fexu_auth::provider::verify_local_password
```

This localizes the observed execution path. It does **not** prove a bug in Argon2, Rust, AVX2, hardware, or Fexu. Reproduction outside Fexu rules out a requirement for Fexu's database/session/router code in this observed failure; it is not a general correctness proof of those components. No root cause or production fix has been established. Compare on an independent Windows host and preserve the precise compiler, CPU, binary, PDB and dependency versions before assigning upstream ownership.

## Capture a private dump and stack

With Windows SDK Debugging Tools (`cdb.exe`) installed, launch the debug EXE under CDB, disable the debugger heap with `-hd`, and configure a **second-chance** access-violation command:

```text
sxd -c2 ".dump /ma C:/private/argon2-crash.dmp; .ecxr; kv; .lastevent; qd" av; g
```

Use `cdb -hd -g -G -c <command> <executable> 24 8388608`. Create a private destination directory first. CDB may itself return0 after executing the capture command, so a created dump/second-chance event is a failure even when the debugger exit code is0. A completed test summary is required to call an auth test run successful.

Open the dump using `cdb -z <dump> -c "!analyze -v; .exr -1; .ecxr; kv; q"` with the exact matching EXE/PDB. Full auth dumps can contain process memory, ephemeral passwords, keys and cookies. Keep them private; never commit them or upload them with public CI artifacts. The captured auth dump and original EXE/PDB remain only in the local private acceptance directory; the repository contains only this code and a limited symbolic stack.

This diagnostic is not part of the production workspace or a replacement for its default parallel tests. It is intentionally a standalone Cargo workspace with a locked dependency set.
