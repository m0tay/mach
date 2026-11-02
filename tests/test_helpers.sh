#!/usr/bin/env bash
# Shared live-suite fixtures. Call only after declaring MACH_TEST_MACHINES.
set -euo pipefail
TEST_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/mach-tests.XXXXXX")
TEST_ID="$(date +%s)-$$"
MACH_DATA_DIR="${MACH_HOME:-$HOME/.mach}"
mkdir -p "$MACH_DATA_DIR/run"
TEST_LOCK="$MACH_DATA_DIR/run/integration-tests.lock"
if ! mkdir "$TEST_LOCK" 2>/dev/null; then
    echo "Another Mach integration suite is running ($TEST_LOCK)." >&2
    rmdir "$TEST_ROOT"
    exit 1
fi
ORIGINAL_DEFAULT="none"
MACH_TEST_MACHINES=()
MACH_TEST_CONTAINER=""
cleanup_test_state() {
    local status=$?
    trap - EXIT
    set +e
    if [[ -n "$MACH_TEST_CONTAINER" ]]; then
        docker --context mach rm -f "$MACH_TEST_CONTAINER" >/dev/null 2>&1
    fi
    # Restore only our default-machine change; preserve independent config edits.
    local current_default restore_default=false
    current_default=$(machctl config get default_machine)
    for machine in ${MACH_TEST_MACHINES[@]+"${MACH_TEST_MACHINES[@]}"}; do
        [[ "$current_default" == "$machine" ]] && restore_default=true
        mach stop "$machine" --force >/dev/null 2>&1
        mach delete "$machine" -y >/dev/null 2>&1
    done
    if $restore_default; then
        machctl config set default_machine "$ORIGINAL_DEFAULT" >/dev/null 2>&1
    fi
    if [[ -n "${HOST_TEST_FILE:-}" ]]; then rm -f "$HOST_TEST_FILE"; fi
    if [[ -n "${GUEST_WRITE_FILE:-}" ]]; then rm -f "$GUEST_WRITE_FILE"; fi
    rm -rf "$TEST_ROOT"
    rmdir "$TEST_LOCK"
    exit "$status"
}
trap cleanup_test_state EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
ORIGINAL_DEFAULT=$(machctl config get default_machine)
assert_redirect() {
    local command=$1 output status=0
    shift
    output=$(mach "$command" "$@" 2>&1) || status=$?
    if [[ $status -ne 0 && "$output" == *"machctl $command"* ]]; then
        report_pass "mach $command redirects to machctl without execution"
    else
        report_fail "mach $command must reject administration (exit=$status): $output"
    fi
}
