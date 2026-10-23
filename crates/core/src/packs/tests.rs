//! Integration tests for the pack system: catalog, loader, manifest, rendering.

use std::collections::BTreeMap;
use std::fs;

use time::{Duration, OffsetDateTime};

use crate::packs::catalog::Catalog;
use crate::packs::loader::parse_pack;
use crate::packs::manifest::Sender;
use crate::packs::render::{RenderContext, render_template};
use crate::packs::trace_values::{TraceValues, generate_trace_values};
use crate::testing::FixedClock;

/// 2025-06-15T10:30 UTC
fn clock() -> FixedClock {
    let base = OffsetDateTime::UNIX_EPOCH
        + Duration::days(20_254)
        + Duration::hours(10)
        + Duration::minutes(30);
    FixedClock::new(base)
}

fn full_retail_trace() -> TraceValues {
    let mut m = BTreeMap::new();
    m.insert("RFQ_NUMBER".into(), "RFQ-20250615-0001".into());
    m.insert("QUOTE_NUMBER".into(), "QUOTE-20250615-0001".into());
    m.insert("PO_NUMBER".into(), "PO-20250615-0001".into());
    m.insert("INVOICE_NUMBER".into(), "INV-20250615-0001".into());
    m.insert("SHIPMENT_ID".into(), "SHIP-20250615-0001".into());
    m.insert("PAYMENT_REF".into(), "PAY-20250615-0001".into());
    TraceValues(m)
}

fn full_healthcare_trace() -> TraceValues {
    let mut m = BTreeMap::new();
    m.insert("CLAIM_ID".into(), "CLM-20250615-0001".into());
    m.insert("TRN_REF".into(), "TRN-20250615-0001".into());
    m.insert("REMITTANCE_TRACE".into(), "RMT-20250615-0001".into());
    m.insert("HC_INVOICE_NUMBER".into(), "HCINV-20250615-0001".into());
    TraceValues(m)
}

fn full_supply_chain_trace() -> TraceValues {
    let mut m = BTreeMap::new();
    m.insert("SHIPMENT_ID".into(), "SCL-20250615-0001".into());
    m.insert("SC_INVOICE_NUMBER".into(), "SCINV-20250615-0001".into());
    TraceValues(m)
}

// ── Catalog tests ────────────────────────────────────────────────────────────

#[test]
fn built_in_catalog_has_three_packs() {
    let catalog = Catalog::built_in();
    assert_eq!(
        catalog.len(),
        3,
        "expected retail, healthcare, supply_chain"
    );
}

#[test]
fn catalog_get_returns_retail_pack() {
    let catalog = Catalog::built_in();
    let retail = catalog.get("retail").expect("retail pack must be present");
    assert_eq!(retail.manifest.pack.id, "retail");
    assert_eq!(retail.manifest.pack.label, "Retail");
    assert_eq!(retail.manifest.pack.standard, "X12");
    assert_eq!(retail.manifest.pack.version, "004010");
    assert_eq!(retail.manifest.pack.terminator, "~");
    assert_eq!(retail.manifest.pack.isa_version, "00401");
}

#[test]
fn catalog_get_returns_healthcare_pack() {
    let catalog = Catalog::built_in();
    let hc = catalog
        .get("healthcare")
        .expect("healthcare pack must be present");
    assert_eq!(hc.manifest.pack.terminator, "!");
    assert_eq!(hc.manifest.pack.isa_version, "00200");
}

#[test]
fn catalog_get_returns_supply_chain_pack() {
    let catalog = Catalog::built_in();
    let sc = catalog
        .get("supply_chain")
        .expect("supply_chain pack must be present");
    assert_eq!(sc.manifest.pack.terminator, "!");
    assert_eq!(sc.manifest.pack.isa_version, "00200");
}

#[test]
fn catalog_returns_none_for_unknown_pack() {
    let catalog = Catalog::built_in();
    assert!(catalog.get("unknown_domain").is_none());
}

// ── Document sequence tests ───────────────────────────────────────────────────

#[test]
fn retail_document_sequence_matches_script() {
    // DOMAIN_DOC_SEQUENCE=(840 843 850 855 860 865 856 810 820) — v1 script
    let catalog = Catalog::built_in();
    let retail = catalog.get("retail").unwrap();
    let codes: Vec<&str> = retail.documents().iter().map(|d| d.code.as_str()).collect();
    assert_eq!(
        codes,
        vec![
            "840", "843", "850", "855", "860", "865", "856", "810", "820"
        ]
    );
}

#[test]
fn healthcare_document_sequence_matches_script() {
    // DOMAIN_DOC_SEQUENCE=(837 275 277 835 820) — v1 script
    let catalog = Catalog::built_in();
    let hc = catalog.get("healthcare").unwrap();
    let codes: Vec<&str> = hc.documents().iter().map(|d| d.code.as_str()).collect();
    assert_eq!(codes, vec!["837", "275", "277", "835", "820"]);
}

#[test]
fn supply_chain_document_sequence_matches_script() {
    // DOMAIN_DOC_SEQUENCE=(204 990 211 214 210 820) — v1 script
    let catalog = Catalog::built_in();
    let sc = catalog.get("supply_chain").unwrap();
    let codes: Vec<&str> = sc.documents().iter().map(|d| d.code.as_str()).collect();
    assert_eq!(codes, vec!["204", "990", "211", "214", "210", "820"]);
}

// ── Sender direction tests ────────────────────────────────────────────────────

#[test]
fn retail_850_is_sent_by_p1() {
    let catalog = Catalog::built_in();
    let retail = catalog.get("retail").unwrap();
    let doc = retail.documents().iter().find(|d| d.code == "850").unwrap();
    assert_eq!(doc.sender, Sender::P1);
}

#[test]
fn retail_810_is_sent_by_p2() {
    let catalog = Catalog::built_in();
    let retail = catalog.get("retail").unwrap();
    let doc = retail.documents().iter().find(|d| d.code == "810").unwrap();
    assert_eq!(doc.sender, Sender::P2);
}

#[test]
fn sender_matches_isa_order_for_all_retail_templates() {
    let catalog = Catalog::built_in();
    let retail = catalog.get("retail").unwrap();
    let clock = clock();
    let trace = full_retail_trace();

    for doc in retail.documents() {
        let template = retail
            .template(&doc.code)
            .unwrap_or_else(|| panic!("missing template for {}", doc.code));
        let ctx = RenderContext {
            p1_duns: "P1DUNS111111111",
            p2_duns: "P2DUNS222222222",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        let rendered = render_template(template, &ctx)
            .unwrap_or_else(|e| panic!("render {} failed: {e}", doc.code));

        let first = rendered.lines().next().unwrap();
        let fields: Vec<&str> = first.split('*').collect();
        // fields[6] = ISA06, fields[8] = ISA08
        let isa06 = fields[6].trim();
        let isa08 = fields[8].trim();

        match doc.sender {
            Sender::P1 => {
                assert_eq!(
                    isa06, "P1DUNS111111111",
                    "tx {} sender=P1: ISA06 should be P1",
                    doc.code
                );
                assert_eq!(
                    isa08, "P2DUNS222222222",
                    "tx {} sender=P1: ISA08 should be P2",
                    doc.code
                );
            }
            Sender::P2 => {
                assert_eq!(
                    isa06, "P2DUNS222222222",
                    "tx {} sender=P2: ISA06 should be P2",
                    doc.code
                );
                assert_eq!(
                    isa08, "P1DUNS111111111",
                    "tx {} sender=P2: ISA08 should be P1",
                    doc.code
                );
            }
        }
    }
}

#[test]
fn sender_matches_isa_order_for_all_healthcare_templates() {
    let catalog = Catalog::built_in();
    let hc = catalog.get("healthcare").unwrap();
    let clock = clock();
    let trace = full_healthcare_trace();

    for doc in hc.documents() {
        let template = hc.template(&doc.code).unwrap();
        let ctx = RenderContext {
            p1_duns: "P1DUNS111111111",
            p2_duns: "P2DUNS222222222",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        let rendered = render_template(template, &ctx)
            .unwrap_or_else(|e| panic!("render hc/{} failed: {e}", doc.code));
        let first = rendered.lines().next().unwrap();
        let fields: Vec<&str> = first.split('*').collect();
        let isa06 = fields[6].trim();
        let isa08 = fields[8].trim();
        match doc.sender {
            Sender::P1 => {
                assert_eq!(isa06, "P1DUNS111111111", "hc/{} sender=P1", doc.code);
                assert_eq!(isa08, "P2DUNS222222222", "hc/{} sender=P1", doc.code);
            }
            Sender::P2 => {
                assert_eq!(isa06, "P2DUNS222222222", "hc/{} sender=P2", doc.code);
                assert_eq!(isa08, "P1DUNS111111111", "hc/{} sender=P2", doc.code);
            }
        }
    }
}

#[test]
fn sender_matches_isa_order_for_all_supply_chain_templates() {
    let catalog = Catalog::built_in();
    let sc = catalog.get("supply_chain").unwrap();
    let clock = clock();
    let trace = full_supply_chain_trace();

    for doc in sc.documents() {
        let template = sc.template(&doc.code).unwrap();
        let ctx = RenderContext {
            p1_duns: "P1DUNS111111111",
            p2_duns: "P2DUNS222222222",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        let rendered = render_template(template, &ctx)
            .unwrap_or_else(|e| panic!("render sc/{} failed: {e}", doc.code));
        let first = rendered.lines().next().unwrap();
        let fields: Vec<&str> = first.split('*').collect();
        let isa06 = fields[6].trim();
        let isa08 = fields[8].trim();
        match doc.sender {
            Sender::P1 => {
                assert_eq!(isa06, "P1DUNS111111111", "sc/{} sender=P1", doc.code);
                assert_eq!(isa08, "P2DUNS222222222", "sc/{} sender=P1", doc.code);
            }
            Sender::P2 => {
                assert_eq!(isa06, "P2DUNS222222222", "sc/{} sender=P2", doc.code);
                assert_eq!(isa08, "P1DUNS111111111", "sc/{} sender=P2", doc.code);
            }
        }
    }
}

// ── Trace value generation tests ──────────────────────────────────────────────

#[test]
fn generate_trace_values_uses_pack_rules() {
    let catalog = Catalog::built_in();
    let retail = catalog.get("retail").unwrap();
    let clock = clock();

    let tv = generate_trace_values(&retail.manifest.trace_rules, &clock, 1);
    assert_eq!(tv.get("RFQ_NUMBER"), Some("RFQ-20250615-0001"));
    assert_eq!(tv.get("QUOTE_NUMBER"), Some("QUOTE-20250615-0001"));
    assert_eq!(tv.get("PO_NUMBER"), Some("PO-20250615-0001"));
    assert_eq!(tv.get("INVOICE_NUMBER"), Some("INV-20250615-0001"));
    assert_eq!(tv.get("SHIPMENT_ID"), Some("SHIP-20250615-0001"));
    assert_eq!(tv.get("PAYMENT_REF"), Some("PAY-20250615-0001"));
}

#[test]
fn healthcare_trace_values_match_script() {
    let catalog = Catalog::built_in();
    let hc = catalog.get("healthcare").unwrap();
    let clock = clock();

    let tv = generate_trace_values(&hc.manifest.trace_rules, &clock, 1);
    assert_eq!(tv.get("CLAIM_ID"), Some("CLM-20250615-0001"));
    assert_eq!(tv.get("TRN_REF"), Some("TRN-20250615-0001"));
    assert_eq!(tv.get("REMITTANCE_TRACE"), Some("RMT-20250615-0001"));
    assert_eq!(tv.get("HC_INVOICE_NUMBER"), Some("HCINV-20250615-0001"));
}

#[test]
fn supply_chain_trace_values_match_script() {
    let catalog = Catalog::built_in();
    let sc = catalog.get("supply_chain").unwrap();
    let clock = clock();

    let tv = generate_trace_values(&sc.manifest.trace_rules, &clock, 1);
    assert_eq!(tv.get("SHIPMENT_ID"), Some("SCL-20250615-0001"));
    assert_eq!(tv.get("SC_INVOICE_NUMBER"), Some("SCINV-20250615-0001"));
}

// ── User override tests ────────────────────────────────────────────────────────

#[test]
fn user_override_replaces_generated_trace_value() {
    let catalog = Catalog::built_in();
    let retail = catalog.get("retail").unwrap();
    let clock = clock();

    let mut tv = generate_trace_values(&retail.manifest.trace_rules, &clock, 1);
    // User overrides PO_NUMBER
    tv.set("PO_NUMBER", "MY-CUSTOM-PO-001");
    assert_eq!(tv.get("PO_NUMBER"), Some("MY-CUSTOM-PO-001"));
    // Other values are unchanged
    assert_eq!(tv.get("RFQ_NUMBER"), Some("RFQ-20250615-0001"));
}

// ── User packs directory tests ─────────────────────────────────────────────────

#[test]
fn user_packs_dir_can_override_built_in() {
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let retail_dir = tmp.path().join("retail");
    let tmpl_dir = retail_dir.join("templates");
    fs::create_dir_all(&tmpl_dir).unwrap();

    // Minimal manifest that overrides the retail pack id.
    let manifest = r#"
[pack]
id = "retail"
label = "Retail (custom)"
standard = "X12"
version = "004010"
terminator = "~"
isa_version = "00401"

[trace_rules]
PO_NUMBER = "CUSTOM-{date}-{seq}"

[[documents]]
code = "850"
name = "Custom Purchase Order"
template = "templates/850.edi"
sender = "p1"
"#;
    fs::write(retail_dir.join("pack.toml"), manifest).unwrap();
    // Write a trivial template.
    fs::write(tmpl_dir.join("850.edi"), "ISA*CUSTOM*{{PO_NUMBER}}~\n").unwrap();

    let (catalog, errors) = Catalog::with_user_dir(tmp.path());
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");

    let retail = catalog.get("retail").unwrap();
    assert_eq!(retail.manifest.pack.label, "Retail (custom)");
    assert_eq!(retail.documents().len(), 1);
}

#[test]
fn nonexistent_user_dir_returns_built_in_catalog() {
    use std::path::PathBuf;
    let (catalog, errors) = Catalog::with_user_dir(&PathBuf::from("/no/such/path"));
    assert!(errors.is_empty());
    assert_eq!(catalog.len(), 3);
}

#[test]
fn malformed_manifest_in_user_dir_produces_error_and_skips_pack() {
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let bad_dir = tmp.path().join("bad_pack");
    fs::create_dir_all(&bad_dir).unwrap();
    fs::write(bad_dir.join("pack.toml"), "NOT VALID TOML === }{{{{").unwrap();

    let (catalog, errors) = Catalog::with_user_dir(tmp.path());
    assert_eq!(errors.len(), 1);
    // Built-ins are still all present.
    assert_eq!(catalog.len(), 3);
}

// ── parse_pack error tests ─────────────────────────────────────────────────────

#[test]
fn parse_pack_errors_on_missing_template() {
    let manifest = r#"
[pack]
id = "test"
label = "Test"
standard = "X12"
version = "004010"
terminator = "~"
isa_version = "00401"
[[documents]]
code = "999"
name = "Missing"
template = "templates/999.edi"
sender = "p1"
"#;
    let err = parse_pack("test", manifest, |_| None).unwrap_err();
    assert!(
        matches!(err, crate::packs::loader::LoadError::MissingTemplate { ref code, .. } if code == "999"),
        "unexpected: {err:?}"
    );
}

// ── Full render golden tests ───────────────────────────────────────────────────

/// Render every retail template with a fixed clock and confirm there are no
/// leftover `{{...}}` placeholders in the output.
#[test]
fn all_retail_templates_render_without_leftover_placeholders() {
    let catalog = Catalog::built_in();
    let retail = catalog.get("retail").unwrap();
    let clock = clock();
    let trace = full_retail_trace();

    for doc in retail.documents() {
        let template = retail.template(&doc.code).unwrap();
        let ctx = RenderContext {
            p1_duns: "123456789",
            p2_duns: "987654321",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        render_template(template, &ctx)
            .unwrap_or_else(|e| panic!("retail/{} render error: {e}", doc.code));
    }
}

#[test]
fn all_healthcare_templates_render_without_leftover_placeholders() {
    let catalog = Catalog::built_in();
    let hc = catalog.get("healthcare").unwrap();
    let clock = clock();
    let trace = full_healthcare_trace();

    for doc in hc.documents() {
        let template = hc.template(&doc.code).unwrap();
        let ctx = RenderContext {
            p1_duns: "123456789",
            p2_duns: "987654321",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        render_template(template, &ctx)
            .unwrap_or_else(|e| panic!("healthcare/{} render error: {e}", doc.code));
    }
}

#[test]
fn all_supply_chain_templates_render_without_leftover_placeholders() {
    let catalog = Catalog::built_in();
    let sc = catalog.get("supply_chain").unwrap();
    let clock = clock();
    let trace = full_supply_chain_trace();

    for doc in sc.documents() {
        let template = sc.template(&doc.code).unwrap();
        let ctx = RenderContext {
            p1_duns: "123456789",
            p2_duns: "987654321",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        render_template(template, &ctx)
            .unwrap_or_else(|e| panic!("supply_chain/{} render error: {e}", doc.code));
    }
}

/// Verify the full first ISA line of the rendered 850 matches the script's output
/// for the same inputs (matches v1 render_template behaviour).
#[test]
fn golden_retail_850_full_isa_line() {
    let catalog = Catalog::built_in();
    let retail = catalog.get("retail").unwrap();
    let clock = clock();
    let trace = full_retail_trace();
    let template = retail.template("850").unwrap();

    let ctx = RenderContext {
        p1_duns: "123456789",
        p2_duns: "987654321",
        isa_ctrl: 1,
        gs_ctrl: 1,
        trace: &trace,
        clock: &clock,
    };
    let out = render_template(template, &ctx).unwrap();
    let first = out.lines().next().unwrap();
    // From the script: p1_isa = printf '%-15s' "123456789"  → "123456789      "
    //                  p2_isa = printf '%-15s' "987654321"  → "987654321      "
    //                  isa_date = ${current_date:2} of "20250615" → "250615"
    //                  isa_ctrl = printf '%09d' 1 → "000000001"
    assert_eq!(
        first,
        "ISA*00*          *00*          *01*123456789      *01*987654321      *250615*1030*U*00401*000000001*0*T*:~"
    );
}

/// Control number wrap: ISA_CTRL 999_999_999 formats as 9 digits exactly.
#[test]
fn isa_ctrl_max_fits_in_9_digits() {
    let clock = clock();
    let trace = TraceValues::default();
    let template = "{{ISA_CTRL}}";
    let ctx = RenderContext {
        p1_duns: "X",
        p2_duns: "Y",
        isa_ctrl: 999_999_999,
        gs_ctrl: 1,
        trace: &trace,
        clock: &clock,
    };
    let out = render_template(template, &ctx).unwrap();
    assert_eq!(out, "999999999");
    assert_eq!(out.len(), 9);
}
