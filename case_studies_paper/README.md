# Case studies of the Biorouter paper

This folder holds the shareable materials of the two clinical case studies in the Biorouter manuscript
(Gu, Bellucci and Baranzini, *Biorouter: An Agentic-AI-powered Integrated Biomedical Research Environment*).

> **MOCK DATA — for privacy purposes.** The case studies were run on de-identified UCSF electronic health
> records (UCSF OMOP), which cannot be redistributed. This folder therefore contains **no patient data and no
> results derived from patients**: no patient-level records, cohort counts, risk distributions, model estimates,
> calibration parameters, reports or figures of the UCSF analyses. Wherever the calculator app needs data, it
> ships **MOCK** values that were simulated or chosen for illustration and are labelled as such in the code and in
> the app. The aggregate results of the case studies are reported in the manuscript.

## Contents

```
case_studies_paper/
├── MS_hospitalization/
│   └── MS_hospitalization_prompt.txt      Prompt of the MS hospitalization case study (verbatim)
└── PREVENT/
    ├── PREVENT_prompt.txt                 Prompt of the PREVENT validation case study (verbatim)
    └── prevent_calculator_app/            PREVENT calculator built with Biorouter's app builder
        ├── install.sh                     Installs the skill and the app, then opens the app
        ├── skill/prevent-calculator-handoff/SKILL.md
        └── app/prevent-ucsf-risk-calculator/   App source (src/), bundle (dist/), launchers, README
```

The two prompts are the same as Supplementary Data S3 of the manuscript. They contain instructions only and no
patient data. Connector names (UCSFOMOPAgent, SPOKEAgent) and folder names are reproduced as written.

## What is MOCK in the calculator

| Item | Public copy | Original case study (not distributed) |
|---|---|---|
| Reference cohort for the percentile and histograms (`REFERENCE_COHORT` in `src/preventData.ts`) | **MOCK**: 20,000 simulated adults aged 30–79 with plausible PREVENT inputs, fixed random seed, binned risk distribution | Aggregated UCSF validation cohort |
| Calibrated risk (`UCSF_RECALIBRATION`) | **MOCK**: illustrative parameters, about 80% of the PREVENT risk | Cox model fitted on the UCSF cohort |
| Demo patient and demo cohort | Synthetic (already in the original app) | — |
| PREVENT coefficients and OMOP extraction rules | Published coefficients; rules contain codes and time windows only | Same |

In the app, MOCK values are labelled "MOCK" and the calibrated risk is shown as "Calibrated (MOCK)". To use the
calculator with real data, replace the two MOCK constants with aggregated values from your own cohort, following
your institution's disclosure rules, and rebuild with `node build.mjs`.

## Running the calculator

See `PREVENT/prevent_calculator_app/app/prevent-ucsf-risk-calculator/README.md`. In short: install Biorouter
Desktop, then run `bash PREVENT/prevent_calculator_app/install.sh`. Loading patients or cohorts from OMOP requires
the UCSFOMOPAgent extension and your own credentials; without them, the app works with manual inputs and the
synthetic demo patient and cohort.

Research use only; not for clinical decisions.
