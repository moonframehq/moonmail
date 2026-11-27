#!/bin/sh
# Cargo passes the compiler followed by --crate-name for compilation units.
case "$2:$3" in
  --crate-name:moonmail_network|--crate-name:moonmail_store|--crate-name:jmap|--crate-name:stalwart)
    exec "$@" -C instrument-coverage ;;
  *) exec "$@" ;;
esac
