# Case studies of the Biorouter paper

This folder holds the shareable materials of the two clinical case studies in the Biorouter manuscript.

> **Privacy.** The case studies were run on de-identified UCSF electronic health records (UCSF OMOP), which cannot be redistributed. This folder contains **no patient-level data**.
> - The **reports** contain aggregate results only (cohort counts, model performance with confidence intervals, and figures), as reported in the manuscript. Counts from 1 to 10 are suppressed.
> - The **PREVENT calculator** ships **MOCK** reference and calibration data, simulated or chosen for illustration and labelled as such in the code and the app.

## Contents

```
case_studies_paper/
├── MS_hospitalization/
│   ├── MS_hospitalization_prompt.txt      Prompt of the MS hospitalization case study (verbatim)
│   └── report/
│       ├── export/report.html             Case report (aggregate results)
│       └── figures/S_MS1–S_MS4.png        ROC and calibration curves, predictor importance, predictor-set AUROCs
└── PREVENT/
    ├── PREVENT_prompt.txt                 Prompt of the PREVENT validation case study (verbatim)
    ├── report/
    │   ├── export/report.html             Case report: PREVENT validation at UCSF and preventr cross-check
    │   └── figures/F3B.png                UCSF versus Khan et al. (2026): C statistic and calibration slope, overall and by sex
    └── prevent_calculator_app/            PREVENT calculator built with Biorouter's app builder (MOCK data)
        ├── install.sh                     Installs the skill and the app, then opens the app
        ├── skill/prevent-calculator-handoff/SKILL.md
        └── app/prevent-ucsf-risk-calculator/   App source (src/), bundle (dist/), launchers, README
```

- **Prompts.** The two prompts are the same as Supplementary Data S3 of the manuscript. They contain instructions only and no patient data. Connector names (UCSFOMOPAgent, SPOKEAgent) and folder names are reproduced as written.
- **Reports.** Open `report/export/report.html` from its folder so that the figures load. The reports name further aggregate tables (CSV files) that are not distributed here.
- **PREVENT report.** The report is limited to the results reported in the manuscript; an exploratory prediction-model comparison (Module B), which the manuscript does not report, was omitted.

## What is MOCK in the calculator

| Item | Public copy | Original case study (not distributed) |
|---|---|---|
| Reference cohort for the percentile and histograms (`REFERENCE_COHORT` in `src/preventData.ts`) | **MOCK**: 20,000 simulated adults aged 30–79 with plausible PREVENT inputs, fixed random seed, binned risk distribution | Aggregated UCSF validation cohort |
| Calibrated risk (`UCSF_RECALIBRATION`) | **MOCK**: illustrative parameters, about 80% of the PREVENT risk | Cox model fitted on the UCSF cohort |
| Demo patient and demo cohort | Synthetic (already in the original app) | — |
| PREVENT coefficients and OMOP extraction rules | Published coefficients; rules contain codes and time windows only | Same |

In the app, MOCK values are labelled "MOCK", and the calibrated risk is shown as "Calibrated (MOCK)". To use the calculator with real data, replace the two MOCK constants with aggregated values from your own cohort, following your institution's disclosure rules, and rebuild with `node build.mjs`.

## Running the calculator

See `PREVENT/prevent_calculator_app/app/prevent-ucsf-risk-calculator/README.md`. In short:
1. Install Biorouter Desktop.
2. Run `bash PREVENT/prevent_calculator_app/install.sh`.

Loading patients or cohorts from OMOP requires the UCSFOMOPAgent extension and your own credentials. Without them, the app works with manual inputs and with the synthetic demo patient and cohort.

Research use only; not for clinical decisions.
