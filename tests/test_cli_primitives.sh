#!/usr/bin/env bash
# End-to-end integration test suite for Mach CLI primitives
set -eo pipefail

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
    if echo "$output" | grep -q "$expected"; then
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
assert_contains "$VER_OUT" "0.4.0" "mach version returns semantic version"

HELP_OUT=$(machctl --help)
assert_contains "$HELP_OUT" "Linux machines:" "machctl --help lists Linux machines group"
assert_contains "$HELP_OUT" "Containers:" "machctl --help lists Containers group"
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

# 4. Status checks on empty environment
echo -e "\n${YELLOW}[4/15] Testing docker and k8s status commands...${NC}"
DOCKER_ST=$(mach docker status)
assert_contains "$DOCKER_ST" "Checking mach Docker status" "mach docker status executes"
assert_contains "$DOCKER_ST" "machctl" "mach docker status provides polite guidance"

DOCKER_CTL_ST=$(machctl docker status)
assert_contains "$DOCKER_CTL_ST" "Checking mach Docker status" "machctl docker status executes directly"

K8S_ST=$(mach k8s status)
assert_contains "$K8S_ST" "Kubernetes" "mach k8s status reports cleanly"
assert_contains "$K8S_ST" "machctl" "mach k8s status provides polite guidance"

K8S_CTL_ST=$(machctl k8s status)
assert_contains "$K8S_CTL_ST" "Kubernetes" "machctl k8s status reports cleanly directly"

# 5. Machine creation (Alpine)
echo -e "\n${YELLOW}[5/15] Testing machine creation (Alpine)...${NC}"
CREATE_OUT=$(mach create alpine test-alp --cpus 2 --memory 1024 --disk 5)
assert_contains "$CREATE_OUT" "created successfully" "mach create alpine succeeds"

LS_OUT=$(mach ls)
assert_contains "$LS_OUT" "test-alp" "mach ls shows newly created machine"

# 6. Machine metadata info, inspect, dump, and default
echo -e "\n${YELLOW}[6/15] Testing machine info, inspect, dump, and default setting...${NC}"
INFO_OUT=$(mach info test-alp)
assert_contains "$INFO_OUT" "test-alp" "mach info shows machine name"
assert_contains "$INFO_OUT" "1024 MiB" "mach info shows allocated memory"

if [ -f "$HOME/.mach/machines/test-alp/machine.yaml" ]; then
    report_pass "Machine metadata persisted in machine.yaml"
else
    report_fail "Machine metadata machine.yaml not found"
fi

INSPECT_YAML=$(machctl inspect test-alp)
assert_contains "$INSPECT_YAML" "name: test-alp" "machctl inspect outputs YAML by default"
assert_contains "$INSPECT_YAML" "hardware:" "machctl inspect includes hardware section"

INSPECT_JSON=$(machctl inspect test-alp --json)
assert_contains "$INSPECT_JSON" '"name": "test-alp"' "machctl inspect --json outputs valid JSON"

DUMP_YAML=$(machctl dump test-alp)
assert_contains "$DUMP_YAML" "name: test-alp" "machctl dump outputs YAML by default"

DUMP_JSON=$(machctl dump test-alp --json)
assert_contains "$DUMP_JSON" '"name": "test-alp"' "machctl dump --json outputs valid JSON"

MACH_INSPECT_ERR=$(mach inspect test-alp 2>&1 || true)
assert_contains "$MACH_INSPECT_ERR" "In mach, 'machctl' manages the macOS virtualization control plane" "mach inspect redirects to machctl"

DEF_OUT=$(mach default test-alp)
assert_contains "$DEF_OUT" "test-alp" "mach default sets default machine"

# 7. APFS Copy-on-Write Clone
echo -e "\n${YELLOW}[7/15] Testing APFS Copy-on-Write clone...${NC}"
CLONE_OUT=$(machctl clone test-alp test-cloned)
assert_contains "$CLONE_OUT" "cloned" "machctl clone creates clone"
LS_CLONE=$(mach ls)
assert_contains "$LS_CLONE" "test-cloned" "mach ls shows cloned machine"

# 8. Rename machine
echo -e "\n${YELLOW}[8/15] Testing machine rename...${NC}"
RENAME_OUT=$(machctl rename test-cloned test-renamed)
assert_contains "$RENAME_OUT" "Renamed" "machctl rename succeeds"
LS_RENAME=$(mach ls)
assert_contains "$LS_RENAME" "test-renamed" "mach ls shows renamed machine"

# 9. Machine deletion
echo -e "\n${YELLOW}[9/15] Testing machine deletion...${NC}"
DEL_OUT=$(mach delete test-renamed -y)
assert_contains "$DEL_OUT" "deleted successfully" "mach delete succeeds with -y"

# 10. Raw disk Export and Import
echo -e "\n${YELLOW}[10/15] Testing disk export and import...${NC}"
EXPORT_FILE="/tmp/mach_test_alp.raw"
rm -f "$EXPORT_FILE"
EXPORT_OUT=$(machctl export test-alp "$EXPORT_FILE")
assert_contains "$EXPORT_OUT" "exported" "machctl export saves raw disk"
if [ -f "$EXPORT_FILE" ]; then
    report_pass "Exported disk file exists on host"
else
    report_fail "Exported disk file does not exist on host"
fi

IMPORT_OUT=$(machctl import "$EXPORT_FILE" test-imported)
assert_contains "$IMPORT_OUT" "Imported" "machctl import creates new machine from raw disk"
mach delete test-imported -y > /dev/null
rm -f "$EXPORT_FILE"
report_pass "Imported machine deleted and export artifact cleaned up"

# 11. Boot Alpine VM
echo -e "\n${YELLOW}[11/15] Testing VM boot and DHCP/SSH connection...${NC}"
START_OUT=$(mach start test-alp)
assert_contains "$START_OUT" "test-alp" "mach start boots VM and acquires DHCP/SSH"

# 12. Run commands inside guest
echo -e "\n${YELLOW}[12/15] Testing command execution inside Linux guest...${NC}"
RUN_OUT=$(mach run test-alp uname -a)
assert_contains "$RUN_OUT" "Linux" "mach run executes uname inside VM"

# Transparent command mode (using default machine)
TRANS_OUT=$(mach uname -a)
assert_contains "$TRANS_OUT" "Linux" "Transparent 'mach <cmd>' executes in default machine"

# 13. File Push and Pull
echo -e "\n${YELLOW}[13/15] Testing file push and pull...${NC}"
TEST_SECRET="Mach-Integration-Token-$(date +%s)"
echo "$TEST_SECRET" > /tmp/mach_host_sample.txt
mach push -m test-alp /tmp/mach_host_sample.txt /tmp/guest_sample.txt
GUEST_CONTENT=$(mach run test-alp cat /tmp/guest_sample.txt)
assert_contains "$GUEST_CONTENT" "$TEST_SECRET" "mach push transfers file into guest"

rm -f /tmp/mach_pulled_sample.txt
mach pull -m test-alp /tmp/guest_sample.txt /tmp/mach_pulled_sample.txt
PULLED_CONTENT=$(cat /tmp/mach_pulled_sample.txt)
assert_contains "$PULLED_CONTENT" "$TEST_SECRET" "mach pull retrieves file from guest"
rm -f /tmp/mach_host_sample.txt /tmp/mach_pulled_sample.txt

# 14. SSH details
echo -e "\n${YELLOW}[14/15] Testing mach ssh details...${NC}"
SSH_DETAILS=$(mach ssh test-alp --details)
assert_contains "$SSH_DETAILS" "root@" "mach ssh --details outputs SSH connection string"

# 15. Stop VM and Clean Up
echo -e "\n${YELLOW}[15/15] Testing VM stop and final cleanup...${NC}"
STOP_OUT=$(mach stop test-alp)
assert_contains "$STOP_OUT" "stopped successfully" "mach stop shuts down VM"
DEL_FINAL=$(mach delete test-alp -y)
assert_contains "$DEL_FINAL" "deleted successfully" "Final test machine deleted"

echo -e "\n${BLUE}======================================================${NC}"
echo -e "Test Results: ${GREEN}${PASS_COUNT} Passed${NC}, ${RED}${FAIL_COUNT} Failed${NC}"
echo -e "${BLUE}======================================================${NC}"

if [ "$FAIL_COUNT" -gt 0 ]; then
    exit 1
fi
