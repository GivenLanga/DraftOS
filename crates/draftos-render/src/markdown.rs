//! LIR → Markdown. Faithful, plain, and easy to diff in tests.

use draftos_core::lir::*;

pub fn render_markdown(doc: &LirDocument) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", doc.meta.title));

    if !doc.parties.is_empty() {
        out.push_str("**Between:**\n\n");
        for p in &doc.parties {
            let mut line = format!("- **{}** (the \"{}\")", p.name, p.role);
            if let Some(reg) = &p.reg_no {
                line.push_str(&format!(", registration number {reg}"));
            }
            out.push_str(&line);
            out.push('\n');
        }
        out.push('\n');
    }

    for r in &doc.recitals {
        out.push_str(&blocks_md(&r.body, 0));
        out.push('\n');
    }

    for c in &doc.clauses {
        clause_md(&mut out, c, 2);
    }

    if !doc.schedules.is_empty() {
        for s in &doc.schedules {
            out.push_str(&format!("## {}\n\n", s.title));
            out.push_str(&blocks_md(&s.body, 0));
            out.push('\n');
        }
    }

    if !doc.execution.blocks.is_empty() {
        out.push_str("## Execution\n\n");
        for b in &doc.execution.blocks {
            out.push_str(&format!(
                "Signed {} \n\n_____________________________\n\n",
                b.signatory_line
            ));
        }
    }

    if let Some(j) = &doc.meta.jurisdiction {
        out.push_str(&format!("\n_Governing law: {j}._\n"));
    }
    out
}

/// One clause and its sub-clauses. Sub-clauses keep their own numbers and
/// nesting rather than being flattened into the parent's prose.
fn clause_md(out: &mut String, c: &LirClause, level: usize) {
    if c.heading.trim().is_empty() {
        // A headingless sub-clause reads as a contract does: the number runs
        // into its own text rather than sitting on a heading line of its own.
        let body = blocks_md(&c.body, 0);
        out.push_str(&format!("{} {}", c.number, body.trim_start()));
        if !body.ends_with("\n\n") {
            out.push('\n');
        }
    } else {
        let hashes = "#".repeat(level.min(6));
        out.push_str(&format!("{hashes} {}. {}\n\n", c.number, c.heading));
        out.push_str(&blocks_md(&c.body, 0));
    }
    out.push('\n');
    for child in &c.children {
        clause_md(out, child, level + 1);
    }
}

fn blocks_md(blocks: &[Block], indent: usize) -> String {
    let pad = "  ".repeat(indent);
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Paragraph { runs } => {
                out.push_str(&pad);
                out.push_str(&runs_md(runs));
                out.push_str("\n\n");
            }
            Block::List { items, .. } => {
                for item in items {
                    out.push_str(&pad);
                    out.push_str("- ");
                    // First block inline after the bullet; rest indented.
                    let rendered = blocks_md(item, indent + 1);
                    out.push_str(rendered.trim_start());
                }
                out.push('\n');
            }
            Block::Table { rows } => {
                for row in rows {
                    let cells: Vec<String> = row
                        .iter()
                        .map(|cell| blocks_md(cell, 0).replace('\n', " ").trim().to_string())
                        .collect();
                    out.push_str(&format!("{pad}| {} |\n", cells.join(" | ")));
                }
                out.push('\n');
            }
            Block::Variable { label, value, .. } => {
                out.push_str(&pad);
                out.push_str(&value.clone().unwrap_or_else(|| format!("[{label}]")));
                out.push_str("\n\n");
            }
        }
    }
    out
}

fn runs_md(runs: &[Run]) -> String {
    runs.iter()
        .map(|r| match r.style {
            RunStyle::Bold => format!("**{}**", r.text),
            RunStyle::Italic => format!("_{}_", r.text),
            RunStyle::Normal => r.text.clone(),
        })
        .collect()
}
