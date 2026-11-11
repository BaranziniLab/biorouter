---
name: prevent-calculator-handoff
description: "Open the PREVENT UCSF Risk Calculator app, and send it a cohort (or one patient) studied in this chat: the app computes the AHA PREVENT 10-year risks (published and calibrated; the public copy uses MOCK calibration and reference data) for every member and shows the cohort and each patient. Load this skill when the user asks to open the PREVENT calculator, or to send, load or analyse a cohort or a patient in it."
---

# Send a cohort or a patient to the PREVENT calculator

The PREVENT UCSF Risk Calculator is a Biorouter app. It computes PREVENT itself, from the 11 inputs of each person. Your job is only to produce those inputs from UCSF OMOP and hand them over. Do not compute PREVENT in the chat, do not estimate treatment effects, and do not add values that are not in the data.

## Open the calculator
When the user asks to open the PREVENT calculator (or before sending data, if it is not open yet), run the app's launcher and hide its output, which contains a one-time link:
`bash "{{CALCULATOR_DIR}}/run.sh" > /dev/null 2>&1`
Then reply in one sentence that the PREVENT UCSF Risk Calculator is open in the browser. Do not print the launcher's output or any link. Do not open the calculator with Agent Drafter's `launch_app` and do not paste an app address: without its one-time token the address shows an empty page.

## Before you start
- The calculator must be open in the user's browser. If it is not, open it as above first.
- Privacy: never print rows, person_id values or person-level values in the chat. Handle rows only inside code, in files you delete afterwards. Results you report in the chat are aggregate counts only, with counts from 1 to 10 shown as "<11".

## Steps
1. Agree the cohort with the user: criteria, codes, as-of date, and the purpose (why this cohort is studied). Show the definition and the cohort size (aggregate) and wait for confirmation.
2. Extract the 11 inputs for every member at the as-of date, following the rules below. For speed, use two jobs: job 1 returns only the person_ids of the cohort; job 2 reads them in code from job 1's result file and puts them in the query as a VALUES list (never printed), then joins measurements, conditions, drugs and observations on person_id. Check job status at most every 2 minutes.
3. In code, write the rows to a CSV with these columns (person_id may be kept; the handoff drops it): `age, sex, sbp, total_cholesterol_mg_dl, hdl_mg_dl, bmi, egfr, diabetes, current_smoker, bp_treatment, statin`. Leave a value empty when it was not found; the calculator excludes that member and says why.
4. Run, from the shell:
   `node ~/.config/biorouter/agent_drafter/prevent-ucsf-risk-calculator/assets/handoff.mjs cohort <file.csv> --name "<short name>" --purpose "<why>" --definition "<criteria and codes>" --as-of <YYYY-MM-DD> --counts '{"cohort_size": <n>}'`
   For one patient: `... handoff.mjs patient <one-row.csv> --key <code> --sources '<json of source and date per input>'`.
   The script deletes the CSV, opens the calculator and delivers the data once over a local connection. It prints only counts.
5. Tell the user what was sent (number of rows, inputs) and that the Cohort tab shows the analysis. Delete any other file that holds rows.

## Inputs from UCSF OMOP (rules of the UCSF validation study)
| Input | Rule |
|---|---|
| age | as-of year minus year_of_birth; year of birth 1935 is top-coded (age 90 or more): leave age empty |
| sex | gender_concept_id 8532 female, 8507 male |
| sbp | LOINC 8480-6, closest value on or before the as-of date within 1 year, mmHg (unit 8876 or none), 50-300 |
| total_cholesterol_mg_dl | LOINC 2093-3 or SNOMED 121868005, closest within 3 years; mmol/L (8753) x 38.67; 20-1000 |
| hdl_mg_dl | LOINC 2085-9 or SNOMED 28036006, closest within 3 years; mmol/L x 38.67; 5-250 |
| bmi | LOINC 39156-5 or SNOMED 60621009, 698094009, closest within 3 years; 10-100 |
| egfr | CKD-EPI 2021 from creatinine LOINC 2160-0, closest within 1 year (µmol/L 8749 x 0.011312; 0.1-25 mg/dL) |
| diabetes | ICD-9-CM 250 or ICD-10-CM E08, E09, E10, E11, E13 on or before the as-of date; or HbA1c of 6.5% or more within 3 years; or an ATC A10 drug started within 365 days |
| bp_treatment | an ATC C02, C03, C07, C08 or C09 drug started within 365 days before the as-of date |
| statin | an ATC C10AA, C10BA or C10BX drug started within 365 days |
| current_smoker | most recent smoking observation within 3 years (current: 903657, 903659, 903660, 903661, 903663, 903664, 903654, 2514534, 2514535, 4021160, 40664492, 40664586, 2108525; not current: 903651, 903653, 42872410); no record = 0 |

When several values share the closest date, use their mean. Do not write the character `%` in SQL (use LEFT(code, n) or CHAR(37)). Start from the cohort's persons and join the large tables to them on person_id.

PREVENT is defined for adults 30-79 without cardiovascular disease; the calculator computes it only within the PREVENT input ranges (SBP 90-180, total cholesterol 130-320, HDL 20-100, BMI 18.5-39.9, eGFR 15-140) and reports the others as excluded.
