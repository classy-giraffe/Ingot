#!/usr/bin/sh
# Ingot live boot (spec 10.3): the live UKI carries root=tmpfs ingot.live
# (or legacy root=live). The stock dracut cmdline hook
# (dracut-cmdline.service, 77dracut-systemd) refuses to continue on a
# root value it does not recognize before any live runtime-root unit
# could run. This hook is sourced by the stock script's source_hook
# cmdline (before its check): mark root=live and root=tmpfs as handled.
# The live runtime root is built by ingot-live-root.service
# (ConditionKernelCommandLine=|root=tmpfs|root=live|ingot.live).
# Sourced, not executed: end with return, never exit.
case "${root:-}" in
    live | tmpfs) rootok=1 ;;
esac
return 0
