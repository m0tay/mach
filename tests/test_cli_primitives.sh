#!/usr/bin/env bash
# End-to-end integration test suite for Mach CLI primitives
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/test_helpers.sh"
ALP="cli-alp-$TEST_ID"
CLONE="cli-clone-$TEST_ID"
RENAMED="cli-renamed-$TEST_ID"
IMPORTED="cli-import-$TEST_ID"
MACH_TEST_MACHINES=("$ALP" "$CLONE" "$RENAMED" "$IMPORTED")

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

PASS_COUNT=0
FAIL_COUNT=0

function report_pass() {
    echo -e "${GREEN}✓ PASS:${NC} $1"
    PASS_COUNT=$((PASS_COUNT + 1))
}

function report_fail() {
    echo -e "${RED}✖ FAIL:${NC} $1"
    FAIL_COUNT=$((FAIL_COUNT + 1))
}

function assert_contains() {
    local output="$1"
    local expected="$2"
    local desc="$3"
    if echo "$output" | grep -Fq -- "$expected"; then
        report_pass "$desc"
    else
        report_fail "$desc (expected '$expected' in output: $output)"
    fi
}

echo -e "${BLUE}======================================================${NC}"
echo -e "${BLUE}  Mach CLI Primitives Comprehensive Test Suite        ${NC}"
echo -e "${BLUE}======================================================${NC}\n"

# 1. Version test
echo -e "${YELLOW}[1/15] Testing version and help output...${NC}"
VER_OUT=$(mach version)
assert_contains "$VER_OUT" "mach" "mach version returns valid header"
CARGO_VER=$(grep '^version = ' "$SCRIPT_DIR/../Cargo.toml" | head -n 1 | cut -d '"' -f 2)
assert_contains "$VER_OUT" "$CARGO_VER" "mach version returns semantic version"
assert_contains "$(mach --version)" "$CARGO_VER" "mach --version matches workspace"
assert_contains "$(machctl --version)" "$CARGO_VER" "machctl --version matches workspace"

HELP_OUT=$(machctl --help)
assert_contains "$HELP_OUT" "Create a new Linux machine" "machctl --help lists machine management"
assert_contains "$HELP_OUT" "Manage Docker engine integration" "machctl --help lists container management"
assert_contains "$HELP_OUT" "uninstall" "machctl --help lists uninstall command"

UNINSTALL_REDIR=$(mach uninstall 2>&1 || true)
assert_contains "$UNINSTALL_REDIR" "machctl uninstall" "mach uninstall provides clean guidance to machctl"

# 2. Doctor test
echo -e "\n${YELLOW}[2/15] Testing system doctor health checks...${NC}"
DOC_OUT=$(machctl doctor)
assert_contains "$DOC_OUT" "Virtualization entitlement present" "Hypervisor entitlement active"
assert_contains "$DOC_OUT" "macOS Apple Silicon" "Host platform identified as Apple Silicon"

# 3. Config management
echo -e "\n${YELLOW}[3/15] Testing configuration get/set...${NC}"
(
export MACH_HOME="$TEST_ROOT/config-only"
machctl config set default_cpus 4
CFG_OUT=$(machctl config show)
assert_contains "$CFG_OUT" '"default_cpus": 4' "machctl config set updates key in JSON"
machctl config set default_cpus 2
CFG_RESET=$(machctl config show)
assert_contains "$CFG_RESET" '"default_cpus": 2' "machctl config set reverts key"

machctl config set docker.was_running true
CFG_DOCKER=$(machctl config show)
assert_contains "$CFG_DOCKER" '"was_running": true' "machctl config set docker.was_running updates state"
machctl config set docker.was_running false

machctl config set k8s.was_running true
CFG_K8S=$(machctl config show)
assert_contains "$CFG_K8S" '"was_running": true' "machctl config set k8s.was_running updates state"
machctl config set k8s.was_running false

machctl config set system_memory_mb 6144
CFG_MEM=$(machctl config get system_memory_mb)
assert_contains "$CFG_MEM" "6144" "machctl config get system_memory_mb returns updated value"
machctl config set system_memory_mb 4096
CFG_MEM_DEF=$(machctl config get system_memory_mb)
assert_contains "$CFG_MEM_DEF" "4096" "machctl config set reverts system_memory_mb to 4096"

)

# 4. Status checks on empty environment
echo -e "\n${YELLOW}[4/15] Testing docker and k8s status commands...${NC}"
assert_redirect k8s status
DOCKER_CTL_ST=$(machctl docker status)
assert_contains "$DOCKER_CTL_ST" "Docker" "machctl docker status executes directly"
K8S_CTL_ST=$(machctl k8s status)
assert_contains "$K8S_CTL_ST" "Kubernetes" "machctl k8s status executes directly"

# 5. Machine creation (Alpine)
echo -e "\n${YELLOW}[5/15] Testing machine creation (Alpine)...${NC}"
CREATE_OUT=$(mach create alpine $ALP --cpus 2 --memory 1024 --disk 5)
assert_contains "$CREATE_OUT" "created successfully" "mach create alpine succeeds"

LS_OUT=$(mach ls)
assert_contains "$LS_OUT" "$ALP" "mach ls shows newly created machine"

# 6. Machine metadata info, inspect, dump, and default
echo -e "\n${YELLOW}[6/15] Testing machine info, inspect, dump, and default setting...${NC}"
INFO_OUT=$(machctl info "$ALP")
assert_contains "$INFO_OUT" "$ALP" "machctl info shows machine name"
assert_contains "$INFO_OUT" "1024 MiB" "machctl info shows allocated memory"

if [ -f "$MACH_DATA_DIR/machines/$ALP/machine.yaml" ]; then
    report_pass "Machine metadata persisted in machine.yaml"
else
    report_fail "Machine metadata machine.yaml not found"
fi

INSPECT_YAML=$(machctl inspect $ALP)
assert_contains "$INSPECT_YAML" "name: $ALP" "machctl inspect outputs YAML by default"
assert_contains "$INSPECT_YAML" "hardware:" "machctl inspect includes hardware section"

INSPECT_JSON=$(machctl inspect $ALP --json)
assert_contains "$INSPECT_JSON" "\"name\": \"$ALP\"" "machctl inspect --json outputs valid JSON"

DUMP_YAML=$(machctl dump $ALP)
assert_contains "$DUMP_YAML" "name: $ALP" "machctl dump outputs YAML by default"

DUMP_JSON=$(machctl dump $ALP --json)
assert_contains "$DUMP_JSON" "\"name\": \"$ALP\"" "machctl dump --json outputs valid JSON"

MACH_INSPECT_ERR=$(mach inspect $ALP 2>&1 || true)
assert_contains "$MACH_INSPECT_ERR" "machctl inspect" "mach inspect redirects to machctl"

DEF_OUT=$(machctl default "$ALP")
assert_contains "$DEF_OUT" "$ALP" "machctl default sets default machine"

# 7. APFS Copy-on-Write Clone
echo -e "\n${YELLOW}[7/15] Testing APFS Copy-on-Write clone...${NC}"
CLONE_OUT=$(machctl clone $ALP $CLONE)
assert_contains "$CLONE_OUT" "cloned" "machctl clone creates clone"
LS_CLONE=$(mach ls)
assert_contains "$LS_CLONE" "$CLONE" "mach ls shows cloned machine"

# 8. Rename machine
echo -e "\n${YELLOW}[8/15] Testing machine rename...${NC}"
RENAME_OUT=$(machctl rename $CLONE $RENAMED)
assert_contains "$RENAME_OUT" "Renamed" "machctl rename succeeds"
LS_RENAME=$(mach ls)
assert_contains "$LS_RENAME" "$RENAMED" "mach ls shows renamed machine"

# 9. Machine deletion
echo -e "\n${YELLOW}[9/15] Testing machine deletion...${NC}"
DEL_OUT=$(mach delete $RENAMED -y)
assert_contains "$DEL_OUT" "deleted successfully" "mach delete succeeds with -y"

# 10. Raw disk Export and Import
echo -e "\n${YELLOW}[10/15] Testing disk export and import...${NC}"
EXPORT_FILE="$TEST_ROOT/export.raw"
rm -f "$EXPORT_FILE"
EXPORT_OUT=$(machctl export $ALP "$EXPORT_FILE")
assert_contains "$EXPORT_OUT" "exported" "machctl export saves raw disk"
if [ -f "$EXPORT_FILE" ]; then
    report_pass "Exported disk file exists on host"
else
    report_fail "Exported disk file does not exist on host"
fi

IMPORT_OUT=$(machctl import "$EXPORT_FILE" $IMPORTED)
assert_contains "$IMPORT_OUT" "Imported" "machctl import creates new machine from raw disk"
mach delete $IMPORTED -y > /dev/null
rm -f "$EXPORT_FILE"
report_pass "Imported machine deleted and export artifact cleaned up"

# 11. Boot Alpine VM
echo -e "\n${YELLOW}[11/15] Testing VM boot and DHCP/SSH connection...${NC}"
START_OUT=$(mach start $ALP)
assert_contains "$START_OUT" "$ALP" "mach start boots VM and acquires DHCP/SSH"

# 12. Run commands inside guest
echo -e "\n${YELLOW}[12/15] Testing command execution inside Linux guest...${NC}"
RUN_OUT=$(mach -m "$ALP" uname -a)
assert_contains "$RUN_OUT" "Linux" "mach -m executes uname inside VM"

# Transparent command mode (using default machine)
TRANS_OUT=$(mach uname -a)
assert_contains "$TRANS_OUT" "Linux" "Transparent 'mach <cmd>' executes in default machine"
ARGV_OUT=$(mach -m "$ALP" printf '%s\n' 'a b' "single'quote" '$HOME' '$(exit 99)' '; exit 98' '*' --help)
EXPECTED_ARGV=$(printf '%s\n' 'a b' "single'quote" '$HOME' '$(exit 99)' '; exit 98' '*' --help)
if [[ "$ARGV_OUT" == "$EXPECTED_ARGV" ]]; then report_pass "Guest arguments remain literal"; else report_fail "Guest argument preservation failed: $ARGV_OUT"; fi
GUEST_STATUS=0
mach -m "$ALP" sh -c 'exit 37' || GUEST_STATUS=$?
if [[ $GUEST_STATUS -eq 37 ]]; then report_pass "Guest exit 37 propagates"; else report_fail "Expected guest exit 37, got $GUEST_STATUS"; fi
GUEST_STATUS=0
mach -m "$ALP" sh -c 'echo x >> /tmp/mach-exit-255; exit 255' || GUEST_STATUS=$?
EXECUTIONS=$(mach -m "$ALP" cat /tmp/mach-exit-255)
if [[ $GUEST_STATUS -eq 255 && "$EXECUTIONS" == x ]]; then report_pass "Guest exit 255 executes exactly once"; else report_fail "Exit 255 retried or lost ($GUEST_STATUS): $EXECUTIONS"; fi


ROOT_UID=$(mach -m "$ALP" -u root id -u)
if [[ "$ROOT_UID" == 0 ]]; then report_pass "Explicit guest user root selected"; else report_fail "Guest -u root output: $ROOT_UID"; fi
GUEST_CWD=$(mach -m "$ALP" pwd)
if [[ "$GUEST_CWD" == "$PWD" ]]; then report_pass "Host working directory preserved"; else report_fail "Expected cwd $PWD, got $GUEST_CWD"; fi
PIPE_TOKEN="stdin-$TEST_ID"
PIPE_OUT=$(printf '%s' "$PIPE_TOKEN" | mach -m "$ALP" cat)
if [[ "$PIPE_OUT" == "$PIPE_TOKEN" ]]; then report_pass "Piped stdin/stdout remains clean"; else report_fail "Unexpected piped output: $PIPE_OUT"; fi
if python3 - "$ALP" <<'PTY'
import errno, os, pty, select, signal, subprocess, sys, time
master, slave = pty.openpty()
proc = subprocess.Popen(['mach', '-m', sys.argv[1], '-u', 'root'], stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
os.close(slave)
time.sleep(1)
os.write(master, b"printf 'MACH_PTY:%s\\n' \"$(id -u)\"; test -t 0 && echo MACH_TTY_OK; exit 23\n")
data = bytearray()
deadline = time.monotonic() + 30
while time.monotonic() < deadline:
    if select.select([master], [], [], 0.2)[0]:
        try:
            chunk = os.read(master, 65536)
        except OSError as e:
            if e.errno == errno.EIO: break
            raise
        if not chunk: break
        data.extend(chunk)
    if proc.poll() is not None: break
if proc.poll() is None:
    os.killpg(proc.pid, signal.SIGTERM)
    proc.wait(timeout=5)
os.close(master)
text = data.decode(errors='replace')
if proc.returncode != 23 or 'MACH_PTY:0' not in text or 'MACH_TTY_OK' not in text:
    print(f'Interactive shell failed (exit {proc.returncode}): {text}', file=sys.stderr)
    sys.exit(1)
PTY
then report_pass "User-only root shell allocates guest PTY and propagates exit"; else report_fail "Interactive root shell PTY failed"; fi

# 13. File Push and Pull
echo -e "\n${YELLOW}[13/15] Testing file push and pull...${NC}"
TEST_SECRET="Mach-Integration-Token-$(date +%s)"
echo "$TEST_SECRET" > "$TEST_ROOT/host-sample.txt"
machctl push -m "$ALP" "$TEST_ROOT/host-sample.txt" /tmp/guest_sample.txt
GUEST_CONTENT=$(mach -m "$ALP" cat /tmp/guest_sample.txt)
assert_contains "$GUEST_CONTENT" "$TEST_SECRET" "machctl push transfers file into guest"

rm -f "$TEST_ROOT/pulled-sample.txt"
machctl pull -m "$ALP" /tmp/guest_sample.txt "$TEST_ROOT/pulled-sample.txt"
PULLED_CONTENT=$(cat "$TEST_ROOT/pulled-sample.txt")
assert_contains "$PULLED_CONTENT" "$TEST_SECRET" "machctl pull retrieves file from guest"
rm -f "$TEST_ROOT/host-sample.txt" "$TEST_ROOT/pulled-sample.txt"

# 14. SSH details
echo -e "\n${YELLOW}[14/15] Testing mach ssh details...${NC}"
SSH_DETAILS=$(machctl ssh "$ALP" --details)
assert_contains "$SSH_DETAILS" "root@" "machctl ssh --details outputs SSH connection string"

# 15. Stop VM and Clean Up
echo -e "\n${YELLOW}[15/15] Testing VM stop and final cleanup...${NC}"
STOP_OUT=$(mach stop $ALP)
assert_contains "$STOP_OUT" "stopped successfully" "mach stop shuts down VM"
if [[ $(machctl config get default_machine) == "$ALP" ]]; then
    machctl config set default_machine "$ORIGINAL_DEFAULT" >/dev/null
fi
DEL_FINAL=$(mach delete $ALP -y)
assert_contains "$DEL_FINAL" "deleted successfully" "Final test machine deleted"

echo -e "\n${BLUE}======================================================${NC}"
echo -e "Test Results: ${GREEN}${PASS_COUNT} Passed${NC}, ${RED}${FAIL_COUNT} Failed${NC}"
echo -e "${BLUE}======================================================${NC}"

if [ "$FAIL_COUNT" -gt 0 ]; then
    exit 1
fi
