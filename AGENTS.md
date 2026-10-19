# AGENTS.md

## What is mintnix?

Mintnix is a Linux-focused fork of drg-mint-notag that has been updated to build
on the latest Rust stable release.

## Agent Instructions

1. If a prompt contains questions, stop after answering the questions so that
   the user has a chance to reply before you edit any code.
2. Do not add new dependencies without the user's explicit approval.
3. Do not add unsafe code without the user's explicit approval.
4. The user regularly works in Git in the background; do not expect Git status
   to stay the same between turns.
5. The project has a `.envrc` file with `use flake`, so the session
   automatically runs under the development shell.
6. The `justfile` contains recipes that wrap common `cargo` invocations and
   enter the development shell automatically if necessary.
7. Run `just validate` after code changes.
