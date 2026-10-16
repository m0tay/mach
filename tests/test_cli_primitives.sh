#!/usr/bin/env bash
# End-to-end integration test suite for Norb CLI primitives
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
echo -e "${BLUE}  Norb CLI Primitives Comprehensive Test Suite       ${NC}"
echo -e "${BLUE}======================================================${NC}\n"

# 1. Version test
echo -e "${YELLOW}[1/15] Testing version and help output...${NC}"
VER_OUT=$(norb version)
assert_contains "$VER_OUT" "Norb" "norb version returns valid header"
assert_contains "$VER_OUT" "0.1.0" "norb version returns semantic version"

HELP_OUT=$(norbctl --help)
assert_contains "$HELP_OUT" "Linux machines:" "norbctl --help lists Linux machines group"
assert_contains "$HELP_OUT" "Containers:" "norbctl --help lists Containers group"

# 2. Doctor test
echo -e "\n${YELLOW}[2/15] Testing system doctor health checks...${NC}"
DOC_OUT=$(norb doctor)
assert_contains "$DOC_OUT" "Virtualization entitlement present" "Hypervisor entitlement active"
assert_contains "$DOC_OUT" "macOS Apple Silicon" "Host platform identified as Apple Silicon"

# 3. Config management
echo -e "\n${YELLOW}[3/15] Testing configuration get/set...${NC}"
norb config set default_cpus 4
CFG_OUT=$(norb config show)
assert_contains "$CFG_OUT" '"default_cpus": 4' "norb config set updates key in JSON"
norb config set default_cpus 2
CFG_RESET=$(norb config show)
assert_contains "$CFG_RESET" '"default_cpus": 2' "norb config set reverts key"

# 4. Status checks on empty environment
echo -e "\n${YELLOW}[4/15] Testing docker and k8s status commands...${NC}"
DOCKER_ST=$(norb docker status)
assert_contains "$DOCKER_ST" "Checking Norb Docker status" "norb docker status executes"

K8S_ST=$(norb k8s status)
assert_contains "$K8S_ST" "Kubernetes is not installed" "norb k8s status reports cleanly"

# 5. Machine creation (Alpine)
echo -e "\n${YELLOW}[5/15] Testing machine creation (Alpine)...${NC}"
CREATE_OUT=$(norb create alpine test-alp --cpus 2 --memory 1024 --disk 5)
assert_contains "$CREATE_OUT" "created successfully" "norb create alpine succeeds"

LS_OUT=$(norb ls)
assert_contains "$LS_OUT" "test-alp" "norb ls shows newly created machine"

# 6. Machine metadata info and default
echo -e "\n${YELLOW}[6/15] Testing machine info and default setting...${NC}"
INFO_OUT=$(norb info test-alp)
assert_contains "$INFO_OUT" "test-alp" "norb info shows machine name"
assert_contains "$INFO_OUT" "1024 MiB" "norb info shows allocated memory"

DEF_OUT=$(norb default test-alp)
assert_contains "$DEF_OUT" "test-alp" "norb default sets default machine"

# 7. APFS Copy-on-Write Clone
echo -e "\n${YELLOW}[7/15] Testing APFS Copy-on-Write clone...${NC}"
CLONE_OUT=$(norb clone test-alp test-cloned)
assert_contains "$CLONE_OUT" "cloned" "norb clone creates clone"
LS_CLONE=$(norb ls)
assert_contains "$LS_CLONE" "test-cloned" "norb ls shows cloned machine"

# 8. Rename machine
echo -e "\n${YELLOW}[8/15] Testing machine rename...${NC}"
RENAME_OUT=$(norb rename test-cloned test-renamed)
assert_contains "$RENAME_OUT" "Renamed" "norb rename succeeds"
LS_RENAME=$(norb ls)
assert_contains "$LS_RENAME" "test-renamed" "norb ls shows renamed machine"

# 9. Machine deletion
echo -e "\n${YELLOW}[9/15] Testing machine deletion...${NC}"
DEL_OUT=$(norb delete test-renamed -y)
assert_contains "$DEL_OUT" "deleted successfully" "norb delete succeeds with -y"

# 10. Raw disk Export and Import
echo -e "\n${YELLOW}[10/15] Testing disk export and import...${NC}"
EXPORT_FILE="/tmp/norb_test_alp.raw"
rm -f "$EXPORT_FILE"
EXPORT_OUT=$(norb export test-alp "$EXPORT_FILE")
assert_contains "$EXPORT_OUT" "exported" "norb export saves raw disk"
if [ -f "$EXPORT_FILE" ]; then
    report_pass "Exported disk file exists on host"
else
    report_fail "Exported disk file does not exist on host"
fi

IMPORT_OUT=$(norb import "$EXPORT_FILE" test-imported)
assert_contains "$IMPORT_OUT" "Imported" "norb import creates new machine from raw disk"
norb delete test-imported -y > /dev/null
rm -f "$EXPORT_FILE"
report_pass "Imported machine deleted and export artifact cleaned up"

# 11. Boot Alpine VM
echo -e "\n${YELLOW}[11/15] Testing VM boot and DHCP/SSH connection...${NC}"
START_OUT=$(norb start test-alp)
assert_contains "$START_OUT" "test-alp" "norb start boots VM and acquires DHCP/SSH"

# 12. Run commands inside guest
echo -e "\n${YELLOW}[12/15] Testing command execution inside Linux guest...${NC}"
RUN_OUT=$(norb run test-alp uname -a)
assert_contains "$RUN_OUT" "Linux" "norb run executes uname inside VM"

# Transparent command mode (using default machine)
TRANS_OUT=$(norb uname -a)
assert_contains "$TRANS_OUT" "Linux" "Transparent 'norb <cmd>' executes in default machine"

# 13. File Push and Pull
echo -e "\n${YELLOW}[13/15] Testing file push and pull...${NC}"
TEST_SECRET="Norb-Integration-Token-$(date +%s)"
echo "$TEST_SECRET" > /tmp/norb_host_sample.txt
norb push -m test-alp /tmp/norb_host_sample.txt /tmp/guest_sample.txt
GUEST_CONTENT=$(norb run test-alp cat /tmp/guest_sample.txt)
assert_contains "$GUEST_CONTENT" "$TEST_SECRET" "norb push transfers file into guest"

rm -f /tmp/norb_pulled_sample.txt
norb pull -m test-alp /tmp/guest_sample.txt /tmp/norb_pulled_sample.txt
PULLED_CONTENT=$(cat /tmp/norb_pulled_sample.txt)
assert_contains "$PULLED_CONTENT" "$TEST_SECRET" "norb pull retrieves file from guest"
rm -f /tmp/norb_host_sample.txt /tmp/norb_pulled_sample.txt

# 14. SSH details
echo -e "\n${YELLOW}[14/15] Testing norb ssh details...${NC}"
SSH_DETAILS=$(norb ssh test-alp --details)
assert_contains "$SSH_DETAILS" "ssh root@" "norb ssh --details outputs SSH connection string"

# 15. Stop VM and Clean Up
echo -e "\n${YELLOW}[15/15] Testing VM stop and final cleanup...${NC}"
STOP_OUT=$(norb stop test-alp)
assert_contains "$STOP_OUT" "stopped successfully" "norb stop shuts down VM"
DEL_FINAL=$(norb delete test-alp -y)
assert_contains "$DEL_FINAL" "deleted successfully" "Final test machine deleted"

echo -e "\n${BLUE}======================================================${NC}"
echo -e "Test Results: ${GREEN}${PASS_COUNT} Passed${NC}, ${RED}${FAIL_COUNT} Failed${NC}"
echo -e "${BLUE}======================================================${NC}"

if [ "$FAIL_COUNT" -gt 0 ]; then
    exit 1
fi
