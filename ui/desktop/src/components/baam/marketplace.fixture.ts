// Shared test fixture, deliberately NOT a `.test.ts` file: importing one test
// file from another re-registers its suites in the importer.
import type { RegistryExtension, RegistrySkill } from './registry';

/**
 * Seven skill rows copied VERBATIM from `landing/registry.json` at 7c96d796 —
 * the registry the 2026-09-10 composer QA run measured finding F5 against — and
 * kept in that document's order, because registry order is the tie-break a
 * ranked search falls back to and the order an empty query browses in.
 *
 * Frozen here rather than read from `registry.fallback.json`, so the exact
 * rankings the tests pin cannot move when the registry gains a skill. PR #242
 * pins the model-facing Rust matcher against the same seven rows, so the two
 * searches can be compared row for row.
 */
export const MARKETPLACE_SKILLS: RegistrySkill[] = [
  {
    id: 'scientific-visual-communication',
    name: 'Scientific Visual Communication',
    category: 'Core',
    type: 'User-invocable · /scientific-visual-communication',
    description:
      'Plans schematics, posters, slides, figure panels, infographics, visual abstracts, and source-to-visual traceability.',
    tags: ['Visuals', 'Posters', 'Apache-2.0'],
    keywords: [
      'scientific-visual-communication',
      'schematics',
      'infographics',
      'posters',
      'slides',
      'visual',
      'abstracts',
      'apache',
    ],
    download:
      'https://github.com/BaranziniLab/biorouter-skills/releases/download/skill-scientific-visual-communication/scientific-visual-communication.zip',
    filename: 'scientific-visual-communication.zip',
    license: 'Apache-2.0',
  },
  {
    id: 'ggplot-visualization',
    name: 'ggplot2 Visualization',
    category: 'Core',
    type: 'Auto-applied · R plotting',
    description: 'Applies ggplot2 best-practice style when writing R plotting code.',
    tags: ['R', 'ggplot2'],
    keywords: [],
    download:
      'https://github.com/BaranziniLab/biorouter-skills/releases/download/skill-ggplot-visualization/ggplot-visualization.zip',
    filename: 'ggplot-visualization.zip',
    license: 'Apache-2.0',
  },
  {
    id: 'r-scripting',
    name: 'R Scripting',
    category: 'Core',
    type: 'Auto-applied · R code',
    description:
      'Applies tidyverse conventions and documentation standards when writing or reviewing R code.',
    tags: ['R', 'Tidyverse'],
    keywords: [],
    download:
      'https://github.com/BaranziniLab/biorouter-skills/releases/download/skill-r-scripting/r-scripting.zip',
    filename: 'r-scripting.zip',
    license: 'Apache-2.0',
  },
  {
    id: 'python-scripting',
    name: 'Python Scripting',
    category: 'Core',
    type: 'Auto-applied · Python code',
    description:
      'Applies Python naming, typing, error handling, and project structure conventions when writing Python code.',
    tags: ['Python'],
    keywords: [],
    download:
      'https://github.com/BaranziniLab/biorouter-skills/releases/download/skill-python-scripting/python-scripting.zip',
    filename: 'python-scripting.zip',
    license: 'Apache-2.0',
  },
  {
    id: 'clinical-biostatistics',
    name: 'Clinical Biostatistics',
    category: 'Biomedical',
    type: '6 skills · auto-applied',
    description: 'Survival, mixed models, and clinical-trial statistical analysis.',
    tags: ['survival', 'R', 'lme4'],
    keywords: ['clinical-biostatistics', 'survival', 'r', 'lme4'],
    download:
      'https://github.com/BaranziniLab/biorouter-skills/releases/download/skill-clinical-biostatistics/clinical-biostatistics.zip',
    filename: 'clinical-biostatistics.zip',
    license: 'Apache-2.0',
  },
  {
    id: 'data-visualization',
    name: 'Data Visualization',
    category: 'Biomedical',
    type: '13 skills · auto-applied',
    description: 'Publication-quality plots: heatmaps, volcano, Manhattan, dimplots.',
    tags: ['ggplot2', 'matplotlib', 'ComplexHeatmap'],
    keywords: ['data-visualization', 'ggplot2', 'matplotlib', 'complexheatmap'],
    download:
      'https://github.com/BaranziniLab/biorouter-skills/releases/download/skill-data-visualization/data-visualization.zip',
    filename: 'data-visualization.zip',
    license: 'Apache-2.0',
  },
  {
    id: 'single-cell',
    name: 'Single-cell',
    category: 'Biomedical',
    type: '14 skills · auto-applied',
    description: 'scRNA-seq clustering, annotation, trajectory, and integration.',
    tags: ['Scanpy', 'Seurat', 'scVI'],
    keywords: ['single-cell', 'scanpy', 'seurat', 'scvi'],
    download:
      'https://github.com/BaranziniLab/biorouter-skills/releases/download/skill-single-cell/single-cell.zip',
    filename: 'single-cell.zip',
    license: 'Apache-2.0',
  },
];

/**
 * Four extension rows copied VERBATIM from `landing/registry.json` at 7c96d796,
 * in that document's order: three about knowledge graphs, and one private row
 * that is about none.
 */
export const MARKETPLACE_EXTENSIONS: RegistryExtension[] = [
  {
    id: 'cdwagent',
    name: 'CDWAgent',
    organization: 'BaranziniLab · UCSF',
    version: 'v0.5.1',
    description:
      'Multimodal access to the UCSF Clinical Data Warehouse through natural language. One-call cohort building across diagnoses, medications, procedures, labs, radiology/imaging, immunizations, allergies, and vitals; clinical-notes/NLP search; read-only queries, schema discovery, and structured results. Requires UCSF network credentials (CAMPUS\\username and password).',
    tags: ['UCSF', 'MCP', 'CDW', 'Clinical'],
    github: 'https://github.com/BaranziniLab/CDWAgent',
    download:
      'https://github.com/BaranziniLab/CDWAgent/releases/download/v0.5.1-brxt/cdwagent.brxt',
    filename: 'cdwagent.brxt',
    license: 'Apache-2.0',
    privacy: 'private',
    extension_name: 'cdwagent',
    affiliation: ['ucsf'],
  },
  {
    id: 'spokeagent',
    name: 'SPOKEAgent',
    organization: 'BaranziniLab · UCSF',
    version: 'v0.4.1',
    description:
      'Structure-aware access to the SPOKE biomedical knowledge graph (43M nodes): live schema introspection, entity/identifier resolution, node profiling, shortest-path finding, and guarded read-only Cypher across diseases, genes, proteins, drugs, and pathways. Includes a bundled spoke-knowledge-graph skill. Requires a SPOKEAGENT_PASSCODE (see credentials page).',
    tags: ['UCSF', 'MCP', 'Knowledge Graph'],
    github: 'https://github.com/BaranziniLab/SPOKEAgent',
    download:
      'https://github.com/BaranziniLab/SPOKEAgent/releases/download/v0.4.1/spokeagent-0.4.1.brxt',
    filename: 'spokeagent-0.4.1.brxt',
    license: 'Apache-2.0',
    privacy: 'public',
    extension_name: 'spokeagent',
  },
  {
    id: 'codegraphagent',
    name: 'CodeGraph Agent',
    organization: 'Broccolito · UCSF',
    version: 'v0.1.0',
    description:
      'Pre-indexed code knowledge graph. Ask "who calls X?", "what does Y call?", or "what breaks if I change Z?" across 23 languages including R, Julia, MATLAB, Perl. Vendored fork of CodeGraph on tree-sitter.',
    tags: ['MCP', 'Code Intelligence', 'R', 'Tree-sitter'],
    github: 'https://github.com/Broccolito/CodeGraphAgent',
    download:
      'https://github.com/Broccolito/CodeGraphAgent/releases/download/v0.1.0/codegraphagent.brxt',
    filename: 'codegraphagent.brxt',
    license: 'Apache-2.0',
    privacy: 'public',
  },
  {
    id: 'primekgagent',
    name: 'PrimeKGAgent',
    organization: 'BaranziniLab · BRXT',
    version: 'v0.1.0',
    description:
      'MCP adapter for PrimeKG-style biomedical knowledge graph files and APIs, with guarded metadata lookup and graph-resource summaries.',
    tags: ['MCP', 'PrimeKG', 'Knowledge Graph', 'Biomedical', 'Apache-2.0'],
    github: 'https://github.com/BaranziniLab/PrimeKGAgent',
    download:
      'https://github.com/BaranziniLab/PrimeKGAgent/releases/download/v0.1.0-brxt/primekgagent.brxt',
    filename: 'primekgagent.brxt',
    license: 'Apache-2.0',
    privacy: 'public',
  },
];
