# B2B Trace Demo — Quick Start Guide

A single shell script that provisions IBM webMethods B2B SaaS assets and
submits complete multi-document EDI trace sequences for three domains:
**Retail**, **Healthcare**, and **Supply Chain Logistics**.

Designed for PM, Sales, and Marketing demos of the B2B transaction linking
use-case.

---

## Prerequisites

| Tool | Version | Install |
|------|---------|---------|
| `bash` | ≥ 4.0 | macOS: `brew install bash` · Linux: built-in |
| `curl` | any | macOS: `brew install curl` · Linux: `apt/yum install curl` |
| `jq` | ≥ 1.6 | macOS: `brew install jq` · Linux: `apt/yum install jq` |

> **macOS users**: the system `bash` is version 3. Install bash 5 via Homebrew:
> ```bash
> brew install bash
> # Then run the script explicitly with the newer bash:
> /opt/homebrew/opt/bash/bin/bash demo.sh --domain retail
> ```
> Or update your PATH so `/opt/homebrew/opt/bash/bin` comes first.

---

## Configuration

**Never edit `demo.sh`** — all configuration lives in `demo.env`.

```bash
# One-time setup: copy the example file and fill in your values
cp demo.env.example demo.env
# Now edit demo.env with your B2B instance details
```

Open `demo.env` in any text editor and set:

| Variable | Description |
|----------|-------------|
| `B2B_HOST` | Hostname of the IBM webMethods B2B SaaS instance |
| `MCSP_INSTANCE_ID` | MCSP account UUID (from the IBM Cloud console) |
| `MCSP_API_KEY` | Base64-encoded MCSP API key |
| `RETAIL_P1_NAME` / `RETAIL_P1_DUNS` / `RETAIL_P1_PASSWORD` … | Retail domain partner identity and credentials |
| `HEALTHCARE_*` | Healthcare domain partner identity and credentials |
| `SUPPLY_CHAIN_*` | Supply chain domain partner identity and credentials |

> **Important**: `B2B_HOST` is the **B2B server hostname**, not your local
> machine hostname. It is used to scope the workspace directory so running the
> same script against two different B2B instances creates two independent
> workspaces.
>
> **`demo.env` is in `.gitignore`** — it will never be committed accidentally.
> The safe-to-commit template is `demo.env.example`.

---

## Quick Start

```bash
# Make executable (once)
chmod +x demo.sh

# Full retail trace — provisions all assets, submits 9 documents
./demo.sh --domain retail

# Full healthcare trace
./demo.sh --domain healthcare

# Full supply chain trace
./demo.sh --domain supply_chain

# Run all three domains back-to-back in one command
./demo.sh --all-domains
```

On the **first run** for each domain, the script automatically:
1. Creates two trading partners with domain-specific DUNS identities
2. Creates an HTTP-IN channel and associates it to Partner 1
3. Creates a partner user and associates them to Partner 1
4. Installs all required EDI document types for the domain
5. Submits the complete document sequence with consistent linking attributes

On **subsequent runs**, assets are already provisioned — the script skips
setup and goes straight to submitting a fresh trace with new reference numbers.

---

## Domain Descriptions

### Retail (`--domain retail`)
Simulates a full retail procurement cycle between **AcmeIndustries** (buyer)
and **PreferredSupplier** (supplier):

```
840 RFQ → 843 Quote Response → 850 Purchase Order → 855 PO Ack
       → 860 PO Change → 865 PO Change Ack → 856 Advance Ship Notice
       → 810 Invoice → 820 Payment
```

**Linking attribute**: `RFQNumber` (840 → 843, 850) · `PONumber` (850 → 855, 860, 856, 810) · `InvoiceNumber` (810 → 820)

---

### Healthcare (`--domain healthcare`)
Simulates a healthcare claims processing flow between **HealthcarePayer**
and **HealthcareProvider**:

```
837 Claim → 275 Claim Attachments → 277 Claim Status → 835 Remittance → 820 Payment
```

**Linking attribute**: `ClaimID` (837 → 275, 277) · `TRN_Ref` (275 → 277 → 835) · `RemittanceTrace` (835 → 820)

---

### Supply Chain (`--domain supply_chain`)
Simulates a motor carrier load tender flow between **SCL-Shipper**
and **SCL-Carrier**:

```
204 Load Tender → 990 Response → 211 Bill of Lading → 214 Shipment Status
              → 210 Freight Invoice → 820 Payment
```

**Linking attribute**: `ShipmentID` (204 → 990 → 211 → 214) · `SCInvoiceNumber` (210 → 820)

---

## All Commands

```bash
# Full trace (provision + submit all documents)
./demo.sh --domain retail

# Provision B2B assets only, no document submission
./demo.sh --domain retail --setup-only

# Submit documents only (assets must already be provisioned)
./demo.sh --domain retail --submit-only

# Dry run — render and print all EDI templates, no API calls
./demo.sh --domain retail --dry-run

# Submit only specific transaction codes
./demo.sh --domain retail --submit-only --docs 840,843,850

# Custom delay between document submissions (default: 2 seconds)
./demo.sh --domain retail --delay 5

# Run all three domains in sequence (provisions each if needed)
./demo.sh --all-domains

# All domains — dry run (safe preview, no API calls)
./demo.sh --all-domains --dry-run

# List all domains and their workspace status for the current B2B host
./demo.sh --list-domains

# Show full workspace state for a domain
./demo.sh --status retail

# Reset workspace for a domain (re-provisions from scratch on next run)
./demo.sh --reset retail
```

---

## Workspace Files

The script tracks all provisioned asset IDs, run counters, and control numbers
in a workspace directory scoped to the B2B host:

```
.b2b-workspace/
  dev2205134-m-yqb-c1-b2b-ipaas-dev/    ← sanitized from B2B_HOST
    retail.json
    retail_runs.log
    healthcare.json
    healthcare_runs.log
    supply_chain.json
    supply_chain_runs.log
```

The workspace files are **excluded from git** (see `.gitignore`). Each file
records the hostname and MCSP instance it was created for, so the file is
self-describing if you need to inspect it manually.

---

## Troubleshooting

| Problem | Solution |
|---------|----------|
| `jq: command not found` | Install jq: `brew install jq` or `apt install jq` |
| `declare -A: invalid option` | You are using bash 3 (macOS default). Install bash 4+: `brew install bash` |
| `Token request failed` | Check `MCSP_API_KEY` and `MCSP_INSTANCE_ID` in `demo.env` |
| `Document type already installed` | Normal — the script fetches the existing ID via GET and continues |
| `HTTP 401 on document POST` | Partner user credentials may be wrong. Run `--reset <domain>` and re-provision |
| Assets were already created manually | Run `--status <domain>` to check state, or `--reset <domain>` to start fresh |
| Want to submit a second trace on the same host | Just run `./demo.sh --domain retail` again — a new unique trace will be submitted |
| macOS `declare -A: invalid option` | System bash is v3. Run: `brew install bash` then use `/opt/homebrew/opt/bash/bin/bash demo.sh ...` |

---

## Partner Identities Per Domain

| Domain | Partner 1 | DUNS | Partner 2 | DUNS |
|--------|-----------|------|-----------|------|
| retail | AcmeIndustries | 777888999 | PreferredSupplier | 777444111 |
| healthcare | HealthcarePayer | 9012345720000 | HealthcareProvider | 9088877320000 |
| supply_chain | SCL-Shipper | 8811223344000 | SCL-Carrier | 8899776655000 |

All DUNS IDs are globally unique across domains to prevent identity conflicts.
