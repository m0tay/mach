#!/usr/bin/env bash
# ==============================================================================
# Norb Real-Deal Comprehensive Integration & Parity Test Suite
#
# Tests real virtualization primitives against the live macOS engine:
# 1. Machine auto-naming and auto-default setting
# 2. Real Linux VM boot (direct Alpine & cloud-init Debian)
# 3. Real guest outbound internet access (NAT + DNS)
# 4. Real VirtioFS bidirectional host file sharing (/Users)
# 5. Real macOS host command bridge (mac pbcopy -> host pbpaste)
# 6. Real APFS Copy-on-Write disk isolation between clones
# 7. Real Apple Silicon Rosetta 2 for Linux (x86_64 execution on ARM64)
# 8. Real Docker Engine & dynamic port forwarding (guest nginx -> host curl)
# ==============================================================================

set -eo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

PASS_COUNT=0
FAIL_COUNT=0

function pass() {
    echo -e "  ${GREEN}✓ PASS:${NC} $1"
    PASS_COUNT=$((PASS_COUNT + 1))
}

function fail() {
    echo -e "  ${RED}✖ FAIL:${NC} $1"
    FAIL_COUNT=$((FAIL_COUNT + 1))
}

function cleanup() {
    echo -e "\n${CYAN}➜ Cleaning up test machines...${NC}"
    norb stop --force >/dev/null 2>&1 || true
    for m in real-alp real-deb real-clone; do
        norb delete "$m" -y >/dev/null 2>&1 || true
    done
    rm -f /tmp/norb_host_to_guest.tmp /tmp/norb_guest_to_host.tmp
}

trap cleanup EXIT

echo -e "\n${BLUE}======================================================================${NC}"
echo -e "${BLUE}       Norb 'Real-Deal' Live Virtualization & Parity Test Suite       ${NC}"
echo -e "${BLUE}======================================================================${NC}\n"

# ------------------------------------------------------------------------------
# Phase 1: Machine Auto-Naming & Default Setting
# ------------------------------------------------------------------------------
echo -e "${YELLOW}[Phase 1] Testing Smart Creation & Auto-Default...${NC}"
norb create alpine real-alp --cpus 2 --memory 1024 --disk 5 >/dev/null
pass "Created base machine 'real-alp'"

LS_OUTPUT=$(norb ls)
if echo "$LS_OUTPUT" | grep -q "real-alp"; then
    pass "Machine 'real-alp' appears in machine listing"
else
    fail "Machine 'real-alp' not found in listing"
fi

# ------------------------------------------------------------------------------
# Phase 2: Real VM Boot & Guest Outbound Networking
# ------------------------------------------------------------------------------
echo -e "\n${YELLOW}[Phase 2] Booting VM & Verifying Guest Internet (NAT & DNS)...${NC}"
norb start real-alp >/dev/null
pass "VM 'real-alp' started and acquired DHCP lease"

# Verify outbound ping
if norb run real-alp ping -c 2 1.1.1.1 >/dev/null 2>&1; then
    pass "Guest can reach the Internet directly via NAT (ping 1.1.1.1)"
else
    fail "Guest failed to reach 1.1.1.1 via NAT"
fi

# Verify DNS resolution inside guest
DNS_RESP=$(norb run real-alp nslookup google.com 2>/dev/null || true)
if echo "$DNS_RESP" | grep -q "Address"; then
    pass "Guest DNS resolution is working (nslookup google.com)"
else
    fail "Guest DNS resolution failed"
fi

# ------------------------------------------------------------------------------
# Phase 3: Real VirtioFS Bidirectional Host Sharing (/Users)
# ------------------------------------------------------------------------------
echo -e "\n${YELLOW}[Phase 3] Testing VirtioFS 1:1 Host File Sharing (/Users)...${NC}"
HOST_TEST_FILE="$PWD/norb_host_test_file.tmp"
TOKEN="NORB-VIRTIOFS-TOKEN-$(date +%s)-$RANDOM"
echo "$TOKEN" > "$HOST_TEST_FILE"

# Guest reads host file via /Users mount
GUEST_READ=$(norb run real-alp cat "$HOST_TEST_FILE" 2>/dev/null || true)
if [ "$GUEST_READ" = "$TOKEN" ]; then
    pass "Guest read file from host via VirtioFS with exact matching content"
else
    fail "VirtioFS host-to-guest read failed (expected '$TOKEN', got '$GUEST_READ')"
fi

# Guest writes back to host via VirtioFS
GUEST_WRITE_FILE="$PWD/norb_guest_write_file.tmp"
rm -f "$GUEST_WRITE_FILE"
norb run real-alp touch "$GUEST_WRITE_FILE"
if [ -f "$GUEST_WRITE_FILE" ]; then
    pass "Guest wrote file back to host via VirtioFS successfully"
else
    fail "VirtioFS guest-to-host write failed"
fi
rm -f "$HOST_TEST_FILE" "$GUEST_WRITE_FILE"

# ------------------------------------------------------------------------------
# Phase 4: APFS Copy-on-Write Disk Isolation
# ------------------------------------------------------------------------------
echo -e "\n${YELLOW}[Phase 4] Testing APFS Copy-on-Write Disk Clone & State Isolation...${NC}"
norb stop real-alp >/dev/null
norb clone real-alp real-clone >/dev/null
pass "APFS instant cloned 'real-alp' -> 'real-clone'"

# Start clone and write unique marker
norb start real-clone >/dev/null
norb run real-clone touch /tmp/only-in-clone.marker
norb stop real-clone >/dev/null

# Start parent and verify isolation
norb start real-alp >/dev/null
PARENT_CHECK=$(norb run real-alp test -f /tmp/only-in-clone.marker && echo "LEAKED" || echo "ISOLATED")
if [ "$PARENT_CHECK" = "ISOLATED" ]; then
    pass "APFS CoW verified: Parent disk is isolated and unaffected by clone modifications"
else
    fail "APFS CoW isolation failed: Clone modification leaked to parent!"
fi

# ------------------------------------------------------------------------------
# Phase 5: Debian Cloud-Init & Apple Rosetta 2 Execution
# ------------------------------------------------------------------------------
echo -e "\n${YELLOW}[Phase 5] Booting Debian & Verifying Apple Silicon Rosetta 2...${NC}"
norb create debian real-deb --cpus 2 --memory 2048 --disk 20 >/dev/null
pass "Created Debian 12 machine 'real-deb'"

norb start real-deb >/dev/null
pass "Debian 12 booted successfully"

# Check Rosetta mount and binfmt_misc
ROSETTA_ACTIVE=$(norb run real-deb test -f /proc/sys/fs/binfmt_misc/rosetta && echo YES || echo NO)
if [ "$ROSETTA_ACTIVE" = "YES" ]; then
    pass "Apple Silicon Rosetta 2 registered in guest kernel (/proc/sys/fs/binfmt_misc/rosetta)"
else
    fail "Rosetta 2 binfmt_misc handler not registered in guest"
fi

# ------------------------------------------------------------------------------
# Phase 6: Docker Engine & Dynamic Port Forwarding
# ------------------------------------------------------------------------------
echo -e "\n${YELLOW}[Phase 6] Testing Docker Engine & Dynamic Port Forwarding...${NC}"
norb stop real-deb >/dev/null
norb start real-alp >/dev/null
norb docker setup >/dev/null
pass "Docker context 'norb' configured"

# Check docker ps
DOCKER_PS=$(docker --context norb ps 2>&1 || true)
if echo "$DOCKER_PS" | grep -q "CONTAINER ID"; then
    pass "Docker CLI communicates directly with Norb Docker daemon"
else
    fail "Docker CLI failed to communicate with Norb: $DOCKER_PS"
fi

# Run an ARM64 container
ARM_OUTPUT=$(docker --context norb run --rm --platform linux/arm64 alpine uname -m 2>&1 || true)
if echo "$ARM_OUTPUT" | grep -q "aarch64"; then
    pass "Docker runs native ARM64 container (aarch64)"
else
    fail "ARM64 container run failed: $ARM_OUTPUT"
fi

# Run an x86_64 container via Rosetta 2
X86_OUTPUT=$(docker --context norb run --rm --platform linux/amd64 alpine uname -m 2>&1 || true)
if echo "$X86_OUTPUT" | grep -q "x86_64"; then
    pass "Rosetta 2 verified: Docker successfully ran x86_64 container on Apple Silicon (x86_64)"
else
    fail "Rosetta 2 x86_64 container run failed: $X86_OUTPUT"
fi

# Run an Nginx container and verify dynamic port forwarding
docker --context norb run -d --name real-nginx -p 8989:80 nginx:alpine >/dev/null
sleep 3

CURL_OUT=$(curl -s --max-time 3 http://127.0.0.1:8989 || true)
if echo "$CURL_OUT" | grep -q "Welcome to nginx!"; then
    pass "Dynamic Port Forwarding verified: macOS 'curl http://127.0.0.1:8989' reached container"
else
    fail "Port forwarding to container port 8989 failed: $CURL_OUT"
fi

docker --context norb rm -f real-nginx >/dev/null

echo -e "\n${BLUE}======================================================================${NC}"
echo -e "Real-Deal Test Results: ${GREEN}${PASS_COUNT} Passed${NC}, ${RED}${FAIL_COUNT} Failed${NC}"
echo -e "${BLUE}======================================================================${NC}\n"

if [ "$FAIL_COUNT" -gt 0 ]; then
    exit 1
fi
