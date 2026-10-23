//! EDI template rendering.
//!
//! Mirrors `render_template` in the v1 script (branch `v1-bash`):
//! - ISA control number: 9-digit zero-padded (`{:09}`)  
//! - GS control number: unpadded integer
//! - ISA date: 6-digit `YYMMDD` (last 6 of `YYYYMMDD`)  
//! - DATE: 8-digit `YYYYMMDD`
//! - TIME: 4-digit `HHMM`  
//! - P1_ISA / P2_ISA: DUNS padded/truncated to exactly 15 chars  
//! - P1_GS / P2_GS: DUNS as-is  
//! - DELIVERY_DATE: `{date}+14d`  
//! - All `{{KEY}}` from `TraceValues`

use crate::clock::Clock;
use crate::packs::trace_values::{TraceValues, delivery_date};

/// Inputs for one rendered EDI document.
#[derive(Clone)]
pub struct RenderContext<'a> {
    /// Raw DUNS of partner 1. Padded to 15 chars for ISA06.
    pub p1_duns: &'a str,
    /// Raw DUNS of partner 2. Padded to 15 chars for ISA08.
    pub p2_duns: &'a str,
    /// ISA control number (ISA13). Formatted as 9 digits.
    pub isa_ctrl: u32,
    /// GS control number (GS06). Formatted without padding.
    pub gs_ctrl: u32,
    /// Resolved trace values for `{{PLACEHOLDER}}` substitution.
    pub trace: &'a TraceValues,
    /// Time source.
    pub clock: &'a dyn Clock,
}

/// Errors that can occur during template rendering.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("unknown template placeholder: {{{{{0}}}}}")]
    UnknownPlaceholder(String),
}

/// Render an EDI template string into a fully resolved EDI document.
///
/// The `template` string should be the raw content of an `.edi` file. Every
/// `{{KEY}}` is replaced; unrecognised keys produce [`RenderError::UnknownPlaceholder`].
///
/// # Panics
///
/// Does not panic.
pub fn render_template(template: &str, ctx: &RenderContext<'_>) -> Result<String, RenderError> {
    use time::OffsetDateTime;

    let now: OffsetDateTime = ctx.clock.now();

    let date_str = format!(
        "{:04}{:02}{:02}",
        now.year(),
        u8::from(now.month()),
        now.day()
    );
    // ISA_DATE is YYMMDD (the script uses `${current_date:2}`, i.e. characters
    // from index 2 of YYYYMMDD = the last 6 chars).
    let isa_date = &date_str[2..];
    let time_str = format!("{:02}{:02}", now.hour(), now.minute());

    // P1_ISA / P2_ISA: left-padded to 15 chars, truncated if longer.
    let p1_isa = format!("{:<15.15}", ctx.p1_duns);
    let p2_isa = format!("{:<15.15}", ctx.p2_duns);
    let p1_gs = ctx.p1_duns;
    let p2_gs = ctx.p2_duns;

    let isa_ctrl_str = format!("{:09}", ctx.isa_ctrl);
    let gs_ctrl_str = ctx.gs_ctrl.to_string();
    let delivery = delivery_date(ctx.clock);

    // Build the substitution table in the same order as sed in the script
    // (L957-978). Specific keys before generic ones so that e.g. ISA_DATE is
    // replaced before DATE (both share "DATE" as a suffix).
    let substitutions: &[(&str, &str)] = &[
        ("{{ISA_CTRL}}", &isa_ctrl_str),
        ("{{GS_CTRL}}", &gs_ctrl_str),
        ("{{ISA_DATE}}", isa_date),
        ("{{DATE}}", &date_str),
        ("{{TIME}}", &time_str),
        ("{{P1_ISA}}", &p1_isa),
        ("{{P2_ISA}}", &p2_isa),
        ("{{P1_GS}}", p1_gs),
        ("{{P2_GS}}", p2_gs),
        ("{{DELIVERY_DATE}}", &delivery),
    ];

    let mut output = template.to_owned();

    // Apply fixed substitutions.
    for (placeholder, value) in substitutions {
        output = output.replace(placeholder, value);
    }

    // Apply trace values.
    for (key, value) in &ctx.trace.0 {
        let placeholder = format!("{{{{{key}}}}}");
        output = output.replace(&placeholder, value);
    }

    // Detect any remaining {{...}} placeholders that were not resolved.
    if let Some(start) = output.find("{{")
        && let Some(end) = output[start..].find("}}")
    {
        let inner = &output[start + 2..start + end];
        return Err(RenderError::UnknownPlaceholder(inner.to_string()));
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use time::{Duration, OffsetDateTime};

    use super::*;
    use crate::packs::trace_values::TraceValues;
    use crate::testing::FixedClock;

    /// 2025-06-15 10:30 UTC
    fn clock_2025() -> FixedClock {
        let base = OffsetDateTime::UNIX_EPOCH
            + Duration::days(20_254)
            + Duration::hours(10)
            + Duration::minutes(30);
        FixedClock::new(base)
    }

    fn empty_trace() -> TraceValues {
        TraceValues::default()
    }

    fn trace_with(pairs: &[(&str, &str)]) -> TraceValues {
        let mut m = BTreeMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.to_string());
        }
        TraceValues(m)
    }

    #[test]
    fn isa_ctrl_is_9_digits_zero_padded() {
        let clock = clock_2025();
        let trace = empty_trace();
        let template = "ISA*00*          *00*          *01*{{P1_ISA}}*01*{{P2_ISA}}*{{ISA_DATE}}*{{TIME}}*U*00401*{{ISA_CTRL}}*0*T*:~";
        let ctx = RenderContext {
            p1_duns: "123456789",
            p2_duns: "987654321",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        let out = render_template(template, &ctx).unwrap();
        // ISA_CTRL at position after last `*` before `*0*T*:`
        assert!(
            out.contains("*000000001*"),
            "ISA_CTRL must be 9 digits: {out}"
        );
    }

    #[test]
    fn p1_isa_padded_to_15_chars() {
        let clock = clock_2025();
        let trace = empty_trace();
        let template = "{{P1_ISA}}|{{P2_ISA}}";
        let ctx = RenderContext {
            p1_duns: "123",
            p2_duns: "456",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        let out = render_template(template, &ctx).unwrap();
        assert_eq!(out, "123            |456            ");
    }

    #[test]
    fn p1_isa_truncated_to_15_chars() {
        let clock = clock_2025();
        let trace = empty_trace();
        let template = "{{P1_ISA}}";
        let ctx = RenderContext {
            p1_duns: "1234567890123456", // 16 chars → truncated to 15
            p2_duns: "X",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        let out = render_template(template, &ctx).unwrap();
        assert_eq!(out.len(), 15);
        assert_eq!(out, "123456789012345");
    }

    #[test]
    fn p1_gs_is_raw_duns() {
        let clock = clock_2025();
        let trace = empty_trace();
        let template = "{{P1_GS}}|{{P2_GS}}";
        let ctx = RenderContext {
            p1_duns: "123456789",
            p2_duns: "987654321",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        let out = render_template(template, &ctx).unwrap();
        assert_eq!(out, "123456789|987654321");
    }

    #[test]
    fn isa_date_is_yymmdd() {
        let clock = clock_2025();
        let trace = empty_trace();
        let template = "{{ISA_DATE}}";
        let ctx = RenderContext {
            p1_duns: "X",
            p2_duns: "Y",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        let out = render_template(template, &ctx).unwrap();
        // 2025-06-15 → 250615
        assert_eq!(out, "250615");
    }

    #[test]
    fn date_is_yyyymmdd() {
        let clock = clock_2025();
        let trace = empty_trace();
        let template = "{{DATE}}";
        let ctx = RenderContext {
            p1_duns: "X",
            p2_duns: "Y",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        let out = render_template(template, &ctx).unwrap();
        assert_eq!(out, "20250615");
    }

    #[test]
    fn time_is_hhmm() {
        let clock = clock_2025();
        let trace = empty_trace();
        let template = "{{TIME}}";
        let ctx = RenderContext {
            p1_duns: "X",
            p2_duns: "Y",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        let out = render_template(template, &ctx).unwrap();
        // 10:30 → 1030
        assert_eq!(out, "1030");
    }

    #[test]
    fn delivery_date_placeholder_is_14_days_ahead() {
        let clock = clock_2025();
        let trace = empty_trace();
        let template = "{{DELIVERY_DATE}}";
        let ctx = RenderContext {
            p1_duns: "X",
            p2_duns: "Y",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        let out = render_template(template, &ctx).unwrap();
        // 2025-06-15 + 14 = 2025-06-29
        assert_eq!(out, "20250629");
    }

    #[test]
    fn trace_values_are_substituted() {
        let clock = clock_2025();
        let trace = trace_with(&[("PO_NUMBER", "PO-20250615-0001")]);
        let template = "BEG**SA*{{PO_NUMBER}}**{{DATE}}~";
        let ctx = RenderContext {
            p1_duns: "X",
            p2_duns: "Y",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        let out = render_template(template, &ctx).unwrap();
        assert_eq!(out, "BEG**SA*PO-20250615-0001**20250615~");
    }

    #[test]
    fn unknown_placeholder_is_an_error() {
        let clock = clock_2025();
        let trace = empty_trace();
        let template = "{{UNKNOWN_KEY}}";
        let ctx = RenderContext {
            p1_duns: "X",
            p2_duns: "Y",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        let err = render_template(template, &ctx).unwrap_err();
        assert!(
            matches!(err, RenderError::UnknownPlaceholder(ref k) if k == "UNKNOWN_KEY"),
            "unexpected error: {err:?}"
        );
    }

    /// Verify P2→P1 direction is preserved by 810 template (ISA starts with P2_ISA).
    #[test]
    fn sender_p2_template_has_p2_in_isa06() {
        let clock = clock_2025();
        let trace = trace_with(&[
            ("INVOICE_NUMBER", "INV-20250615-0001"),
            ("PO_NUMBER", "PO-20250615-0001"),
            ("DELIVERY_DATE", "20250629"),
            ("SHIPMENT_ID", "SHIP-20250615-0001"),
            ("RFQ_NUMBER", "RFQ-20250615-0001"),
        ]);
        // 810 template: ISA starts with {{P2_ISA}}
        let template_810 = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packs/retail/templates/810.edi"
        ));
        let ctx = RenderContext {
            p1_duns: "BUYER111111111",
            p2_duns: "SUPPLIER22222",
            isa_ctrl: 42,
            gs_ctrl: 42,
            trace: &trace,
            clock: &clock,
        };
        let out = render_template(template_810, &ctx).unwrap();
        let first_line = out.lines().next().unwrap();
        // ISA06 (6th element, 0-indexed field 5) should be P2 padded to 15
        let fields: Vec<&str> = first_line.split('*').collect();
        // ISA*00*{ISA_AUTHINFO}*00*{ISA_SECINFO}*{ISA_QUAL06}*{ISA06}*{ISA_QUAL08}*{ISA08}*...
        assert_eq!(fields[6].trim(), "SUPPLIER22222", "ISA06 should be P2 DUNS");
        assert_eq!(
            fields[8].trim(),
            "BUYER111111111",
            "ISA08 should be P1 DUNS"
        );
    }

    /// Golden test: render the 850 template with a fixed clock and known inputs,
    /// compare against the expected output line by line.
    #[test]
    fn golden_850_first_line() {
        let clock = clock_2025();
        let trace = trace_with(&[
            ("RFQ_NUMBER", "RFQ-20250615-0001"),
            ("QUOTE_NUMBER", "QUOTE-20250615-0001"),
            ("PO_NUMBER", "PO-20250615-0001"),
            ("INVOICE_NUMBER", "INV-20250615-0001"),
            ("SHIPMENT_ID", "SHIP-20250615-0001"),
            ("PAYMENT_REF", "PAY-20250615-0001"),
        ]);
        let template_850 = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packs/retail/templates/850.edi"
        ));
        let ctx = RenderContext {
            p1_duns: "123456789",
            p2_duns: "987654321",
            isa_ctrl: 1,
            gs_ctrl: 1,
            trace: &trace,
            clock: &clock,
        };
        let out = render_template(template_850, &ctx).unwrap();
        let first = out.lines().next().unwrap();
        assert_eq!(
            first,
            "ISA*00*          *00*          *01*123456789      *01*987654321      *250615*1030*U*00401*000000001*0*T*:~"
        );
    }
}
