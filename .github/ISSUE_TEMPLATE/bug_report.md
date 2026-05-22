---
name: Bug report
about: Something is broken or producing wrong output
title: ''
labels: bug
assignees: ''
---

## Describe the bug

A clear description of what went wrong.

## Reproduction

```bash
# Exact command you ran
shard init
shard add myfile.bin
shard commit -m "test"
```

## Expected vs actual

```
Expected: ...
Actual: ...
```

## Environment

- OS: [e.g. Ubuntu 24.04, macOS 15.2, Windows 11]
- Installation: [cargo install / pre-built binary / built from source]
- Version: `shard --version`
- Network: [NAT type, firewall status if relevant]

## Additional context

- Does the issue reproduce with `--json` output?
- Are there any peers involved in the workflow?
