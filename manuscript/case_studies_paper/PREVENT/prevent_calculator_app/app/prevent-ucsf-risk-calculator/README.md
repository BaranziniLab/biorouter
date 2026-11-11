# PREVENT UCSF Risk Calculator (public version with MOCK data)

Version: 0.2.4-mock

> **MOCK DATA — for privacy purposes.** This public copy contains **no patient data**.
> The reference cohort (`REFERENCE_COHORT`) is a simulated population of 20,000 adults, and the
> calibration parameters (`UCSF_RECALIBRATION`) are illustrative values (about 80% of the PREVENT risk).
> Neither was derived from the UCSF case study. Do not interpret the percentiles or calibrated risks shown by
> this copy as UCSF results.

A Biorouter app built with Biorouter's app builder (Agent Drafter) for the PREVENT case study of the Biorouter
manuscript. It implements the American Heart Association PREVENT base equations for 10-year risk of total
cardiovascular disease (CVD), atherosclerotic CVD (ASCVD) and heart failure (HF), and places a patient or a cohort
against a reference distribution.

## What it does

- Editable patient form for the 11 PREVENT inputs, with instant recalculation of the three risks.
- Percentile and histogram position against the reference cohort (MOCK in this copy), with a sex filter.
- What-if changes for modifiable inputs, up to 3 saved scenarios, and projection by future age.
- Optional OMOP patient loading through the user's own UCSFOMOPAgent credentials.
- Cohort view: a cohort studied in a Biorouter chat is sent to the app with `assets/handoff.mjs`
  (Biorouter skill `prevent-calculator-handoff`), or the built-in synthetic demo cohort is loaded.

## Installation

1. Install Biorouter Desktop.
2. Optional: enable the UCSFOMOPAgent extension with your own credentials to load patients or cohorts from OMOP.
3. From `prevent_calculator_app/`, run `bash install.sh` (installs the skill and the app, then opens it), or run
   the app's launcher directly: `bash run.sh` (macOS/Linux) or `run.bat` (Windows).
4. After changing `src/`, rebuild the bundle with `node build.mjs` (Node.js 22.6 or later; nothing to install).

## Data bundled in the app (`src/preventData.ts`)

| Constant | Content in this public copy |
|---|---|
| `PREVENT_COEFFICIENTS` | Published PREVENT coefficients (Khan SS et al., Circulation 2024; Supplemental Table S12.A) |
| `REFERENCE_COHORT` | **MOCK**: binned risk distribution of 20,000 simulated adults (fixed seed; common bin edges, ≥11 simulated persons per bin) |
| `UCSF_RECALIBRATION` | **MOCK**: illustrative parameters (beta = 1, x_mean = 0, s0_10y = exp(−0.8)) |
| `OMOP_RULES` | Rules used to derive the PREVENT inputs from an OMOP database (codes, time windows, units); no patient data |

To use the app locally, replace the two MOCK constants with an aggregated reference distribution and calibration
parameters from your own cohort, following your institution's disclosure rules.

## Privacy

- This repository copy contains no patient-level or patient-derived data.
- When the app is used with OMOP, patient-level values stay in the local app session; cohort results are aggregate,
  and counts from 1 to 10 are displayed as `<11`. The app stores no database credentials.

## Limitations

PREVENT is a population risk model, not a diagnostic tool. This app is for research use only and is not for
clinical decisions.
