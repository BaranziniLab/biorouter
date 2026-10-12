//! Fixture generator for the artifact-panel browser harness.
//!
//! Ignored by default. It drives the real MCP transport — the same duplex path
//! `extension_manager` uses — so the HTML it writes is byte-for-byte what the
//! desktop artifact panel receives at runtime.
//!
//!   PREVIEW_FIXTURE_DIR=/tmp/fixtures cargo test -p biorouter-mcp \
//!     --test preview_fixture_dump -- --ignored --nocapture

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use biorouter_mcp::AgentDrafterServer;
use image::{Rgb, RgbImage};
use rmcp::model::{CallToolRequestParams, RawContent, ResourceContents};
use rmcp::ServiceExt;
use serde_json::json;
use tempfile::TempDir;

fn fixture_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("PREVIEW_FIXTURE_DIR").expect("set PREVIEW_FIXTURE_DIR"))
}

/// Pull the first `ui://` HTML blob out of a tool result.
fn blob_html(result: &rmcp::model::CallToolResult) -> Option<String> {
    for content in &result.content {
        if let RawContent::Resource(embedded) = &content.raw {
            if let ResourceContents::BlobResourceContents { blob, .. } = &embedded.resource {
                return Some(String::from_utf8(STANDARD.decode(blob).unwrap()).unwrap());
            }
        }
    }
    None
}

#[tokio::test]
#[ignore = "fixture generator; writes HTML to $PREVIEW_FIXTURE_DIR"]
async fn dump_agent_drafter_card() {
    let out = fixture_dir();
    std::fs::create_dir_all(&out).unwrap();

    let tmp = TempDir::new().unwrap();
    let server = AgentDrafterServer::with_root(tmp.path().to_path_buf());

    let (server_read, client_write) = tokio::io::duplex(1 << 22);
    let (client_read, server_write) = tokio::io::duplex(1 << 22);
    tokio::spawn(async move {
        if let Ok(running) = server.serve((server_read, server_write)).await {
            let _ = running.waiting().await;
        }
    });
    let client = ().serve((client_read, client_write)).await.unwrap();

    let created = client
        .call_tool(CallToolRequestParams {
            name: "create_app".into(),
            arguments: json!({
                "title": "Cohort Explorer",
                "description": "Ask questions about the trial cohort.",
                "kind": "agentic",
                "system_prompt": "Help researchers explore a paired tumour and normal cohort.",
                "greeting": "What would you like to learn from the cohort?",
                "html": r#"<!doctype html>
                    <html lang="en">
                      <head><meta charset="utf-8"><title>Cohort Explorer</title></head>
                      <body>
                        <main class="br-container">
                          <div class="br-row" style="align-items:end">
                            <div>
                              <div class="br-kicker">Agentic application</div>
                              <h1>Cohort Explorer</h1>
                              <p class="br-muted">48 paired tumour and normal samples are ready.</p>
                            </div>
                            <span class="br-badge br-badge--success">Connected</span>
                          </div>
                          <section class="br-card" aria-labelledby="question-title">
                            <h2 id="question-title">Ask about this cohort</h2>
                            <label class="br-label" for="question">Research question</label>
                            <textarea id="question" class="br-textarea">Which pathways separate tumour from normal tissue?</textarea>
                            <div class="br-row" style="margin-top:12px;align-items:center">
                              <label class="br-switch">
                                <input id="citations" type="checkbox" checked>
                                <span class="br-switch__track"></span>
                                Include source citations
                              </label>
                              <button id="go" class="br-btn" type="button">Summarise cohort</button>
                            </div>
                          </section>
                          <section id="result" class="br-panel" aria-live="polite">
                            Choose a question, then run the agent.
                          </section>
                        </main>
                        <script>
                          const button = document.getElementById('go');
                          const result = document.getElementById('result');
                          button.addEventListener('click', () => {
                            button.disabled = true;
                            button.textContent = 'Analysing…';
                            result.textContent = 'Comparing 24 paired samples…';
                            setTimeout(() => {
                              result.innerHTML = '<strong>MYC signalling is the strongest separator.</strong><p>412 genes pass FDR &lt; 0.05 across the paired analysis.</p>';
                              button.disabled = false;
                              button.textContent = 'Summarise again';
                            }, 120);
                          });
                        </script>
                      </body>
                    </html>"#
            })
            .as_object()
            .cloned(),
            task: None,
            meta: None,
        })
        .await
        .unwrap();

    let html = blob_html(&created).expect("create_app must return a ui:// preview card");
    std::fs::write(out.join("agent-drafter-card.html"), &html).unwrap();
    println!("wrote agent-drafter-card.html ({} bytes)", html.len());

    // `launch_app`/`build_app` also return this card; that is asserted in the
    // agent_drafter unit tests, where an SDK-wired app can pass the lint harness.

    client.cancel().await.unwrap();
}

#[test]
#[ignore = "fixture generator; writes sample files to $PREVIEW_FIXTURE_DIR"]
fn dump_file_previews() {
    let out = fixture_dir();
    std::fs::create_dir_all(&out).unwrap();

    let fixtures = [
        (
            "report.md",
            "# Differential expression report\n\n**412 genes** pass FDR < 0.05.\n\n| Pathway | NES |\n|---|---:|\n| MYC targets | 2.8 |\n| E2F targets | 2.2 |\n",
        ),
        (
            "genes.csv",
            "gene,log2_fold_change,fdr\nMYC,2.81,0.0004\nCDK4,1.92,0.003\n\"HLA-DRA\",-1.44,0.012\n",
        ),
        (
            "config.json",
            "{\n  \"cohort\": \"paired-tumour-normal\",\n  \"fdr\": 0.05,\n  \"replicates\": 24\n}\n",
        ),
        (
            "pipeline.yaml",
            "name: differential-expression\nsteps:\n  - normalize\n  - fit-model\n  - adjust-p-values\n",
        ),
        (
            "sample.xml",
            "<cohort name=\"paired\"><sample id=\"T01\" tissue=\"tumour\" /></cohort>\n",
        ),
        (
            "analysis.R",
            "library(DESeq2)\ndds <- DESeq(dds)\nresults(dds, alpha = 0.05)\n",
        ),
        (
            "summarize.py",
            "from pathlib import Path\n\nprint(Path(\"report.md\").read_text())\n",
        ),
        (
            "query.sql",
            "SELECT gene, log2_fold_change, fdr\nFROM differential_expression\nWHERE fdr < 0.05\nORDER BY fdr;\n",
        ),
        (
            "page.html",
            "<!doctype html><html><body><main><h1>Analysis complete</h1><p>412 genes pass FDR &lt; 0.05.</p></main></body></html>\n",
        ),
        (
            "theme.css",
            ":root { color-scheme: light dark; }\nmain { max-width: 70ch; margin: auto; }\n",
        ),
        (
            "run.sh",
            "#!/usr/bin/env bash\nset -euo pipefail\nRscript analysis.R\n",
        ),
        (
            "notes.txt",
            "Paired design. Adjust for donor and sequencing batch.\n",
        ),
        (
            "Cargo.toml",
            "[package]\nname = \"cohort-analysis\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        ),
        (
            "lib.rs",
            "pub fn significant(fdr: f64) -> bool {\n    fdr < 0.05\n}\n",
        ),
        (
            "app.ts",
            "export const cohort = { pairedSamples: 24, threshold: 0.05 } as const;\n",
        ),
    ];

    for (name, content) in fixtures {
        std::fs::write(out.join(name), content).unwrap();
    }

    let mut image = RgbImage::from_pixel(800, 480, Rgb([250, 248, 243]));
    for x in 60..760 {
        image.put_pixel(x, 420, Rgb([97, 90, 70]));
    }
    for y in 40..421 {
        image.put_pixel(60, y, Rgb([97, 90, 70]));
    }
    for index in 0..96 {
        let x = 78 + (index * 71 % 650);
        let y = 70 + (index * 137 % 320);
        let colour = if index % 5 == 0 {
            Rgb([207, 109, 71])
        } else {
            Rgb([97, 90, 70])
        };
        for dx in 0..5 {
            for dy in 0..5 {
                image.put_pixel(x + dx, y + dy, colour);
            }
        }
    }
    image.save(out.join("volcano.png")).unwrap();

    // A Jupyter notebook fixture that exercises every branch of NotebookPreview:
    // markdown + code + raw cells, a stdout stream, an HTML DataFrame, a plain
    // execute_result, an application/json result, an image/png plot, and an
    // error traceback. The image is a real (small) volcano-style scatter encoded
    // in-memory to PNG, so the image output renders a genuine figure rather than
    // a 1x1 placeholder.
    let mut plot = RgbImage::from_pixel(480, 320, Rgb([255, 253, 248]));
    for x in 40..450 {
        plot.put_pixel(x, 280, Rgb([120, 116, 108]));
    }
    for y in 30..281 {
        plot.put_pixel(40, y, Rgb([120, 116, 108]));
    }
    for index in 0..140u32 {
        let x = 55 + (index * 53 % 380);
        let y = 45 + (index * 97 % 220);
        let colour = if index % 7 == 0 {
            Rgb([207, 109, 71])
        } else {
            Rgb([120, 140, 170])
        };
        for dx in 0..4 {
            for dy in 0..4 {
                if x + dx < 480 && y + dy < 320 {
                    plot.put_pixel(x + dx, y + dy, colour);
                }
            }
        }
    }
    let mut png = std::io::Cursor::new(Vec::<u8>::new());
    image::DynamicImage::ImageRgb8(plot)
        .write_to(&mut png, image::ImageOutputFormat::Png)
        .unwrap();
    let plot_png_b64 = STANDARD.encode(png.into_inner());

    let notebook = json!({
        "cells": [
            {
                "cell_type": "markdown",
                "metadata": {},
                "source": [
                    "# Differential expression walkthrough\n",
                    "\n",
                    "Compare **paired tumour and normal** samples: load the per-gene\n",
                    "statistics, flag hits at `FDR < 0.05`, and plot the volcano.\n",
                    "See [`report.md`](report.md) for the written summary."
                ]
            },
            {
                "cell_type": "code",
                "execution_count": 1,
                "metadata": {},
                "source": [
                    "import numpy as np\n",
                    "import pandas as pd\n",
                    "\n",
                    "genes = pd.read_csv(\"genes.csv\")\n",
                    "print(f\"Loaded {len(genes)} genes across {genes.shape[1]} columns\")"
                ],
                "outputs": [
                    {
                        "output_type": "stream",
                        "name": "stdout",
                        "text": ["Loaded 3 genes across 3 columns\n"]
                    }
                ]
            },
            {
                "cell_type": "code",
                "execution_count": 2,
                "metadata": {},
                "source": [
                    "hits = genes[genes[\"fdr\"] < 0.05].sort_values(\"fdr\")\n",
                    "hits"
                ],
                "outputs": [
                    {
                        "output_type": "execute_result",
                        "execution_count": 2,
                        "metadata": {},
                        "data": {
                            "text/html": [
                                "<table border=\"1\" class=\"dataframe\">\n",
                                "  <thead>\n",
                                "    <tr style=\"text-align: right;\">\n",
                                "      <th></th><th>gene</th><th>log2_fold_change</th><th>fdr</th>\n",
                                "    </tr>\n",
                                "  </thead>\n",
                                "  <tbody>\n",
                                "    <tr><th>0</th><td>MYC</td><td>2.81</td><td>0.0004</td></tr>\n",
                                "    <tr><th>1</th><td>CDK4</td><td>1.92</td><td>0.0030</td></tr>\n",
                                "  </tbody>\n",
                                "</table>"
                            ],
                            "text/plain": [
                                "   gene  log2_fold_change     fdr\n",
                                "0   MYC              2.81  0.0004\n",
                                "1  CDK4              1.92  0.0030"
                            ]
                        }
                    }
                ]
            },
            {
                "cell_type": "code",
                "execution_count": 3,
                "metadata": {},
                "source": ["hits[\"gene\"].tolist()"],
                "outputs": [
                    {
                        "output_type": "execute_result",
                        "execution_count": 3,
                        "metadata": {},
                        "data": {
                            "text/plain": ["['MYC', 'CDK4']"]
                        }
                    }
                ]
            },
            {
                "cell_type": "code",
                "execution_count": 4,
                "metadata": {},
                "source": [
                    "from IPython.display import JSON\n",
                    "\n",
                    "JSON({\n",
                    "    \"cohort\": \"paired-tumour-normal\",\n",
                    "    \"fdr_threshold\": 0.05,\n",
                    "    \"n_hits\": int((genes[\"fdr\"] < 0.05).sum()),\n",
                    "})"
                ],
                "outputs": [
                    {
                        "output_type": "execute_result",
                        "execution_count": 4,
                        "metadata": {},
                        "data": {
                            "application/json": {
                                "cohort": "paired-tumour-normal",
                                "fdr_threshold": 0.05,
                                "n_hits": 2
                            }
                        }
                    }
                ]
            },
            {
                "cell_type": "markdown",
                "metadata": {},
                "source": ["## Volcano plot\n", "\nSignificant genes are highlighted in orange."]
            },
            {
                "cell_type": "code",
                "execution_count": 5,
                "metadata": {},
                "source": [
                    "import matplotlib.pyplot as plt\n",
                    "\n",
                    "plt.figure(figsize=(5, 3.2))\n",
                    "plt.scatter(genes[\"log2_fold_change\"], -np.log10(genes[\"fdr\"]))\n",
                    "plt.axhline(-np.log10(0.05), ls=\"--\", color=\"grey\")\n",
                    "plt.xlabel(\"log2 fold change\")\n",
                    "plt.ylabel(\"-log10 FDR\")\n",
                    "plt.show()"
                ],
                "outputs": [
                    {
                        "output_type": "display_data",
                        "metadata": {},
                        "data": {
                            "image/png": plot_png_b64,
                            "text/plain": ["<Figure size 500x320 with 1 Axes>"]
                        }
                    }
                ]
            },
            {
                "cell_type": "markdown",
                "metadata": {},
                "source": [
                    "## A cell that fails\n",
                    "\n",
                    "The next cell references a column that does not exist, so it raises."
                ]
            },
            {
                "cell_type": "code",
                "execution_count": 6,
                "metadata": {},
                "source": ["genes[\"pvalue\"]  # column does not exist -> KeyError"],
                "outputs": [
                    {
                        "output_type": "error",
                        "ename": "KeyError",
                        "evalue": "'pvalue'",
                        "traceback": [
                            "---------------------------------------------------------------------------",
                            "KeyError                                  Traceback (most recent call last)",
                            "Cell In[6], line 1",
                            "----> 1 genes[\"pvalue\"]  # column does not exist -> KeyError",
                            "",
                            "File ~/env/lib/python3.11/site-packages/pandas/core/frame.py:3805, in DataFrame.__getitem__(self, key)",
                            "   3804     return self._getitem_multilevel(key)",
                            "-> 3805 indexer = self.columns.get_loc(key)",
                            "",
                            "KeyError: 'pvalue'"
                        ]
                    }
                ]
            },
            {
                "cell_type": "raw",
                "metadata": {},
                "source": [
                    "This is a raw cell: nbconvert passes it through verbatim\n",
                    "(for example a LaTeX or reStructuredText block)."
                ]
            }
        ],
        "metadata": {
            "kernelspec": {
                "display_name": "Python 3 (ipykernel)",
                "language": "python",
                "name": "python3"
            },
            "language_info": {
                "name": "python",
                "version": "3.11.6"
            }
        },
        "nbformat": 4,
        "nbformat_minor": 5
    });
    std::fs::write(
        out.join("notebook.ipynb"),
        serde_json::to_string_pretty(&notebook).unwrap(),
    )
    .unwrap();

    println!("wrote {} file preview fixtures", fixtures.len() + 2);
}
