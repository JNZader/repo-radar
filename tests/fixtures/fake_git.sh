#!/usr/bin/env bash
# Fake git: simulates a successful shallow clone.
# The target directory already exists (created by TempDir::new()).
# Usage: fake_git.sh clone --depth 1 --single-branch <url> <dir>
exit 0
