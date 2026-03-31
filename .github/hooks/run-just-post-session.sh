#!/usr/bin/env bash
set -euo pipefail

payload="$(cat)"

# Run only after file-modifying tool calls.
if [[ "$payload" != *'replace_string_in_file'* && \
	  "$payload" != *'multi_replace_string_in_file'* && \
	  "$payload" != *'create_file'* && \
	  "$payload" != *'edit_notebook_file'* ]]; then
	exit 0
fi

just fmt
just lint