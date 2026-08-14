# IBM webMethods B2B Toolkit

A single-script toolkit for IBM webMethods B2B SaaS. Use `--feature` to select
the capability you need. All configuration lives in `toolkit.env` — the script
itself is never edited.

Designed for PM, Sales, and Marketing demos of B2B SaaS capabilities including
partner provisioning and multi-document EDI transaction tracing.

---

## Features

| `--feature` | What it does |
|-------------|--------------|
| `setup` | Provisions B2B assets for a domain — trading partners, HTTP-IN channels, partner users, and EDI document types. Idempotent: safe to run multiple times against the same instance. |
| `trace` | Provisions assets (if not already done) then submits a complete, linked multi-document EDI trace sequence for the chosen domain. |

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
> # Then run the script with the newer bash:
> /opt/homebrew/opt/bash/bin/bash b2b-toolkit.sh --feature setup --domain retail
> ```
> Or update your PATH so `/opt/homebrew/opt/bash/bin` comes first.

---

## Configuration

**Never edit `b2b-toolkit.sh`** — all configuration lives in `toolkit.env`.

```bash
# One-time setup: copy the example file and fill in your values
cp toolkit.env.example toolkit.env
# Now edit toolkit.env with your B2B instance details
```

Open `toolkit.env` in any text editor and set:

| Variable | Description |
|----------|-------------|
| `B2B_HOST` | Hostname of the IBM webMethods B2B SaaS instance |
| `MCSP_INSTANCE_ID` | MCSP account UUID (from the IBM Cloud console) |
| `MCSP_API_KEY` | Base64-encoded MCSP API key |
| `RETAIL_P1_NAME` / `RETAIL_P1_DUNS` / `RETAIL_P1_PASSWORD` … | Retail domain partner identity and credentials |
| `HEALTHCARE_*` | Healthcare domain partner identity and credentials |
| `SUPPLY_CHAIN_*` | Supply chain domain partner identity and credentials |

> `toolkit.env` is in `.gitignore` — it will never be committed accidentally.
> The safe-to-commit template is `toolkit.env.example`.

---

## Obtaining MCSP Credentials

The toolkit authenticates against IBM webMethods B2B SaaS via the **Multi-Cloud
Service Platform (MCSP)** IAM layer. You need two values: the **Instance ID**
and an **API Key**. Both are obtained from the IBM SaaS Console.

### Step 1 — Find your Instance ID (`MCSP_INSTANCE_ID`)

1. Sign in to the **IBM SaaS Console**:
   `https://aws.console.saas.ibm.com`
2. Select your right **SaaS Account**
3. In account page select right **Subscription** of **IBM webMethods Hybrid Integration** and click **View instances**
4. In the subscription page click on correct **Instance**
5. Copy the **Instance ID** from **Instance details** tab (a UUID in the format
   `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`).

This value maps to `MCSP_INSTANCE_ID` in `toolkit.env`.

> **Tip**: the Instance ID also appears in the URL of the B2B Admin Console —
> it is the UUID segment after `/instances/`.

---

### Step 2 — Generate an API Key (`MCSP_API_KEY`)

API keys are managed per-instance in the SaaS Console.

1. From the instance detail page (Step 1), click the **Access** tab (or
   **API Keys** in the left sidebar, depending on console version).
2. Click **Generate API key** (or **Create API key**).
3. Give the key a descriptive name, e.g. `b2b-toolkit-local`.
4. Set the **scope** to the instance you identified in Step 1.
5. Click **Generate**.
6. **Copy the key immediately** — it is shown only once and paste in toolkit.env.


> **Security note**: treat the API key like a password. Store it only in
> `toolkit.env` (which is `.gitignore`d) and never commit or share it.

---

### Step 3 — Find your B2B Host (`B2B_HOST`)

1. In the IBM SaaS Console, open your **IBM webMethods Hybrid Integration** instance.
2. Click **Open** (or **Open Instance**) to open the IBM webMethods Hybrid Integration UI.
3. From IBM webMethods Hybrid Integration left naviagtion panel or from capability section click on ** webMethods B2B Integration**
3. The hostname in your browser's address bar is your `B2B_HOST` value —
   the part between `https://` and the first `/`, for example:
   ```
   dev2205134-m-yqb-c1.b2b.ipaas.ibm.com
   ```
4. Paste this hostname (without `https://`) into `toolkit.env`.

---

### Summary — `toolkit.env` values from MCSP

| Variable | Where to find it |
|----------|-----------------|
| `B2B_HOST` | Browser address bar when the B2B Admin Console is open |
| `MCSP_INSTANCE_ID` | IBM SaaS Console → My instances → Instance detail page |
| `MCSP_API_KEY` | IBM SaaS Console → Instance → Access → Generate API key (copy and paste as-is) |

---

## Quick Start

```bash
# Make executable (once)
chmod +x b2b-toolkit.sh

# ── feature: setup ─────────────────────────────────────────────────────────
# Provision all B2B assets for a domain (partners, channels, doc types)
./b2b-toolkit.sh --feature setup --domain retail
./b2b-toolkit.sh --feature setup --domain healthcare
./b2b-toolkit.sh --feature setup --domain supply_chain

# ── feature: trace ─────────────────────────────────────────────────────────
# Full retail EDI trace — provision assets (if needed) + submit 9 documents
./b2b-toolkit.sh --feature trace --domain retail

# Full healthcare trace
./b2b-toolkit.sh --feature trace --domain healthcare

# Full supply chain trace
./b2b-toolkit.sh --feature trace --domain supply_chain

# Run all three trace domains back-to-back
./b2b-toolkit.sh --feature trace --all-domains
```

---

## Feature Reference

### `--feature setup`

Provisions all B2B assets required for a domain:
1. Creates two trading partners with domain-specific DUNS identities
2. Creates HTTP-IN channels and associates them to each partner
3. Creates partner users and associates them to their partners
4. Installs all required EDI document types for the domain

The operation is **idempotent** — re-running it on an already-provisioned
domain is safe and skips work that was already completed.

```bash
./b2b-toolkit.sh --feature setup --domain retail
./b2b-toolkit.sh --feature setup --domain healthcare
./b2b-toolkit.sh --feature setup --domain supply_chain
```

---

### `--feature trace`

Submits a complete linked EDI document sequence to demonstrate B2B transaction
tracing. Automatically provisions assets first if they are not present.

```bash
# Full trace (provision if needed + submit all documents)
./b2b-toolkit.sh --feature trace --domain retail

# Submit only (assets already provisioned via --feature setup)
./b2b-toolkit.sh --feature trace --domain retail --submit-only

# Dry run — render and print all EDI templates, no API calls
./b2b-toolkit.sh --feature trace --domain retail --dry-run

# Submit only specific transaction codes
./b2b-toolkit.sh --feature trace --domain retail --submit-only --docs 840,843,850

# Custom delay between document submissions (default: 2 seconds)
./b2b-toolkit.sh --feature trace --domain retail --delay 5

# Run all three domains in sequence
./b2b-toolkit.sh --feature trace --all-domains

# All domains — dry run
./b2b-toolkit.sh --feature trace --all-domains --dry-run
```

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

**Linking attributes**: `RFQNumber` · `PONumber` · `InvoiceNumber`

---

### Healthcare (`--domain healthcare`)
Simulates a healthcare claims processing flow between **HealthcarePayer**
and **HealthcareProvider**:

```
837 Claim → 275 Claim Attachments → 277 Claim Status → 835 Remittance → 820 Payment
```

**Linking attributes**: `ClaimID` · `TRN_Ref` · `RemittanceTrace`

---

### Supply Chain (`--domain supply_chain`)
Simulates a motor carrier load tender flow between **SCL-Shipper**
and **SCL-Carrier**:

```
204 Load Tender → 990 Response → 211 Bill of Lading → 214 Shipment Status
              → 210 Freight Invoice → 820 Payment
```

**Linking attributes**: `ShipmentID` · `SCInvoiceNumber`

---

## All Commands

```bash
# Provision assets for a domain
./b2b-toolkit.sh --feature setup --domain retail

# Full trace (provision + submit all documents)
./b2b-toolkit.sh --feature trace --domain retail

# Submit documents only (assets must already be provisioned)
./b2b-toolkit.sh --feature trace --domain retail --submit-only

# Dry run — render and print all EDI templates, no API calls
./b2b-toolkit.sh --feature trace --domain retail --dry-run

# Submit only specific transaction codes
./b2b-toolkit.sh --feature trace --domain retail --submit-only --docs 840,843,850

# Custom delay between document submissions (default: 2 seconds)
./b2b-toolkit.sh --feature trace --domain retail --delay 5

# Run all three domains in sequence
./b2b-toolkit.sh --feature trace --all-domains

# All domains — dry run
./b2b-toolkit.sh --feature trace --all-domains --dry-run

# List all domains and their workspace status for the current B2B host
./b2b-toolkit.sh --list-domains

# Show full workspace state for a domain
./b2b-toolkit.sh --status retail

# Reset workspace for a domain (re-provisions from scratch on next run)
./b2b-toolkit.sh --reset retail
```

---

## Workspace Files

The toolkit tracks all provisioned asset IDs, run counters, and control numbers
in a workspace directory scoped to the B2B host:

```
.b2b-toolkit/
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
| `Token request failed` | Check `MCSP_API_KEY` and `MCSP_INSTANCE_ID` in `toolkit.env` |
| `Document type already installed` | Normal — the script fetches the existing ID via GET and continues |
| `HTTP 401 on document POST` | Partner user credentials may be wrong. Run `--reset <domain>` and re-provision |
| Assets were already created manually | Run `--status <domain>` to check state, or `--reset <domain>` to start fresh |
| Want to submit a second trace | Just run `./b2b-toolkit.sh --feature trace --domain retail` again — a new unique trace will be submitted |
| macOS `declare -A: invalid option` | System bash is v3. Run: `brew install bash` then use `/opt/homebrew/opt/bash/bin/bash b2b-toolkit.sh ...` |

---

## Partner Identities Per Domain

| Domain | Partner 1 | DUNS | Partner 2 | DUNS |
|--------|-----------|------|-----------|------|
| retail | AcmeIndustries | 777888900 | PreferredSupplier | 777444100 |
| healthcare | HealthcarePayer | 9012345330000 | HealthcareProvider | 9088877440000 |
| supply_chain | SCL-Shipper | 8811223344000 | SCL-Carrier | 8899776655000 |

All DUNS IDs are globally unique across domains to prevent identity conflicts.
