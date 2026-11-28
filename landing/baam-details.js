/* BAAM: what each extension does, shown when its row is opened.
   Keyed by registry id. baam.js renders an entry only when a visitor opens
   the row, so none of this text is searched or read by the tag facets.
   Checked against each extension's repository, manifest and server code. */
window.BAAM_DETAILS = {
  cdwagent: {
    domain: 'Clinical data',
    what: 'Gives the agent access to the de-identified UCSF Clinical Data Warehouse (Epic Caboodle on SQL Server). It can read the data and cannot change it. It resolves diagnosis, medication, lab and procedure codes, builds a patient cohort in one call across eight data types, pulls the records of each patient, and searches clinical notes by NLP concept, social determinant or exact text. Every SQL statement is checked before it runs, and the schema can be looked up offline from a bundled data dictionary.',
    asks: [
      'How many patients have a multiple sclerosis diagnosis and a brain MRI?',
      'For this cohort, find notes that mention housing instability.',
      'For OMOP person_id 12345, pull lab trends from the CDW.'
    ],
    tools: ['build_cohort', 'query', 'search_note_concepts', 'search_diagnoses_by_code', 'crossmap_patient', 'get_database_overview', 'submit_query_job'],
    needs: 'UCSF network or VPN access, and read permission on the CDW. Your UCSF login as CLINICAL_RECORDS_USERNAME (CAMPUS\\username) and CLINICAL_RECORDS_PASSWORD. uv and Python 3.11 or later.',
    sources: [
      'UCSF Epic Caboodle CDW, de-identified (database CDW_NEW, schema deid_uf)',
      'cTAKES note concepts (note_concepts, note_concepts_sdoh)',
      'Clinical note text and metadata (note_text, note_metadata)',
      'A bundled data dictionary of 139 tables and about 5000 columns',
      'The UCSF OMOP_DEID person table, used only to match a patient across the two databases'
    ]
  },
  ucsfomopagent: {
    domain: 'Clinical data',
    what: 'Runs SQL against the de-identified UCSF clinical database in the OMOP Common Data Model (v5.4 on Microsoft SQL Server, database OMOP_DEID). A query must be a single SELECT or WITH statement, so it can read and cannot change anything. At the start of a session it gives the model the OMOP and T-SQL conventions. It searches OMOP vocabulary concepts so the model does not guess concept IDs, finds lab and vital measurements with sample values for each unit, and reads the live schema.',
    asks: [
      'Find the concept IDs for hemoglobin A1c and show which units are recorded.',
      'How many people have a type 2 diabetes condition and a metformin drug exposure?',
      'List the OMOP tables and describe the columns of the measurement table.'
    ],
    tools: ['query_ucsf_omop', 'search_concepts', 'find_measurement', 'get_omop_schema', 'list_ucsf_omop_tables', 'submit_query_job'],
    needs: 'UCSF network or VPN access, and read permission on OMOP_DEID. Your UCSF network login as CLINICAL_RECORDS_USERNAME and CLINICAL_RECORDS_PASSWORD. uv and Python 3.11 or later.',
    sources: [
      'UCSF OMOP_DEID (OMOP CDM v5.4 on Microsoft SQL Server, schema omop)',
      'OMOP standard vocabularies (the concept tables)'
    ]
  },
  playwrightagent: {
    domain: 'Web & browser',
    what: 'Packages Microsoft\'s @playwright/mcp server, pinned at 0.0.86, as a Biorouter extension started by a small Python launcher through npx. The agent can open pages, click, type, fill forms, handle dialogs, manage tabs and read the structure of a page through accessibility snapshots, so no vision model is needed. The 24 tools are a reviewed set: the server\'s option to run arbitrary code in the page is off.',
    asks: [
      'Open arxiv.org and find papers on retrieval augmented generation from the last week.',
      'Search ClinicalTrials.gov for multiple sclerosis remyelination and summarize the first ten active trials.',
      'Fill out the contact form on example.com with my name and email.'
    ],
    tools: ['browser_navigate', 'browser_snapshot', 'browser_click', 'browser_type', 'browser_fill_form', 'browser_tabs', 'browser_take_screenshot', 'browser_wait_for'],
    needs: 'Node.js 18 or later with npx on PATH, and access to the npm registry the first time it starts. No credentials.',
    sources: ['Any website the browser visits', 'The @playwright/mcp package from npm (Microsoft)']
  },
  spokeagent: {
    domain: 'Knowledge graphs',
    what: 'Queries SPOKE, a Neo4j biomedical knowledge graph of about 43 million nodes that links diseases, genes, proteins, compounds, pathways, anatomy and side effects. It reads the live schema into a compact map of edge types, resolves names, synonyms and identifiers to canonical nodes and checks labels before a query runs, describes the relationships of a node, and finds the shortest paths between two entities. Its Cypher queries can only read, with an automatic LIMIT and a time limit on each transaction, and long queries run as jobs the agent can wait on.',
    asks: [
      'Which genes are associated with Alzheimer\'s disease?',
      'How are metformin and multiple sclerosis connected?',
      'What relationship types does the TNF gene node have, and how many of each?'
    ],
    tools: ['get_spoke_schema', 'resolve_entity', 'describe_node', 'find_path', 'query_spoke'],
    skills: ['spoke-knowledge-graph'],
    needs: 'A passcode from UCSF, from the credentials page below. uv and Python 3.11 or later.',
    sources: [
      'SPOKE knowledge graph (Neo4j, about 43 million nodes)',
      'Identifiers it resolves: DOID, Entrez, Ensembl, DrugBank, UMLS CUI, UBERON and GO'
    ],
    links: [{ label: 'SPOKEAgent credentials page', href: 'https://wiki.library.ucsf.edu/spaces/~Wanjun.Gu@ucsf.edu/pages/755904655/SPOKEAgent+Credentials' }]
  },
  codegraphagent: {
    domain: 'Code',
    what: 'Indexes the project you name into upstream CodeGraph\'s SQLite graph of symbols and call edges, so the agent can explore code, look up a symbol, search and find callers without reading every file. It runs CodeGraph 1.6.4 (MIT) on about 30 languages, including Python, R, TypeScript, C and C++, Rust, Go and Java. It never indexes your home folder, keeps its state apart from a global ~/.codegraph, and sends no telemetry.',
    asks: [
      'Who calls normalize_counts in this R package?',
      'What breaks if I change the signature of load_config?',
      'Show the call path from main to write_results.'
    ],
    tools: ['codegraph_explore', 'codegraph_node', 'codegraph_search', 'codegraph_callers', 'codegraphagent_index_project'],
    needs: 'No credentials. Network access once, to download the CodeGraph 1.6.4 release archive, which is checked against its published SHA-256. It needs uv.',
    sources: ['Your local project source code', 'One index for each project, at <project>/.biorouter/codegraph/codegraph.db']
  },
  bioroffice: {
    domain: 'Productivity',
    what: 'Packages the open source OfficeCLI engine (iOfficeAI, Apache-2.0), so the agent can create, read, analyze and edit .docx, .xlsx and .pptx files without Microsoft Office. One tool, officecli, covers document edits, tables, charts, pivot tables, Excel formula evaluation (more than 150 functions) and rendering to HTML or PNG. Four bundled skills (office suite, Word, Excel, PowerPoint) load at the start of a session. More specific ones, such as academic-paper, pitch-deck and financial-model, load when a task needs them.',
    asks: [
      'Turn this results table into an Excel workbook with a bar chart.',
      'Write a Word document from my notes with headings, a table of contents and footnotes.',
      'Make a PowerPoint of six slides that summarizes this analysis, with speaker notes.'
    ],
    tools: ['officecli'],
    toolsNote: 'Its commands include create, view, get, query, set, add, remove, move, swap, validate and batch.',
    needs: 'No credentials and no Microsoft Office. OfficeCLI 1.0.163 ships inside the bundle for macOS on Apple silicon, with its licence and notices. Other platforms download the same release the first time it starts, checked against its published SHA-256. It needs uv.',
    sources: ['Local .docx, .xlsx and .pptx files']
  },
  benchlingagent: {
    domain: 'Lab & ELN',
    what: 'Built on the BRXT adapter kit: every request goes to a fixed address, and a key is sent only to its own service. It reaches your Benchling tenant\'s REST API v2: projects, notebook entries, sequences, registry entities and inventory. benchling_get only reads, so it runs without asking; benchling_write changes data, so Biorouter asks you before each call.',
    asks: [
      'List the Benchling projects I can see.',
      'Look up the registry entities in this project and list their IDs and owners.',
      'Summarize this notebook entry export and keep its IDs and timestamps.'
    ],
    tools: ['benchling_status', 'benchling_get', 'benchling_write'],
    needs: 'BENCHLING_API_KEY (secret) and BENCHLING_TENANT (the subdomain, for example mylab for https://mylab.benchling.com). The key goes only to your tenant. It needs uv.',
    sources: ['Benchling REST API v2 on your tenant', 'Benchling JSON exports you hand to summarize_benchlingagent_resource']
  },
  dnanexusagent: {
    domain: 'Compute & workflows',
    what: 'Built on the BRXT adapter kit: every request goes to a fixed address, and a key is sent only to its own service. It reaches the DNAnexus platform API. Every DNAnexus call is a POST, so dnanexus_get sends POST only on the routes that read (find, describe, listFolder, getDetails); dnanexus_write changes data, so Biorouter asks you before each call.',
    asks: [
      'Is my DNAnexus token configured, and which API server is this extension using?',
      'Describe project-xxxx and list its owner, creation date and version fields.',
      'Before running anything, tell me whether this DNAnexus job launch would change anything.'
    ],
    tools: ['dnanexus_status', 'dnanexus_get', 'dnanexus_write'],
    skills: ['dnanexus-cloud-analysis'],
    needs: 'DNANEXUS_API_TOKEN (secret), sent only to api.dnanexus.com. It needs uv.',
    sources: ['DNAnexus platform API (api.dnanexus.com)']
  },
  protocolsioagent: {
    domain: 'Lab & ELN',
    what: 'Built on the BRXT adapter kit: every request goes to a fixed address, and a key is sent only to its own service. It searches and reads protocols on protocols.io through the v3 API and keeps their IDs, versions, authors, DOIs and URLs for citation. It is read only.',
    asks: [
      'Search Protocols.io for nuclei isolation protocols for single nucleus RNA-seq.',
      'Fetch protocol 12345 and give me its title, version, authors and DOI for citation.',
      'Summarize this protocol record and list its provenance fields.'
    ],
    tools: ['protocolsio_status', 'protocolsio_get'],
    skills: ['protocols-io-methods'],
    needs: 'PROTOCOLS_IO_TOKEN (secret), which the v3 API requires; it goes only to protocols.io. It needs uv.',
    sources: ['Protocols.io API v3']
  },
  idcagent: {
    domain: 'Imaging',
    what: 'A thin bridge to the official MCP server that NCI Imaging Data Commons hosts. The agent can list collections, attributes, analysis results and clinical tables, build a cohort, get download and viewer URLs, and run SQL over IDC\'s index. If IDC is busy or down when Biorouter starts the extension, the error is reported as a tool result rather than as an empty tool list.',
    asks: [
      'Which IDC collections cover lung cancer, and how many subjects does each one have?',
      'Show the metadata IDC holds for the 4D-Lung collection.',
      'Summarize this IDC cohort export and list the IDs, versions and URLs it contains.'
    ],
    tools: ['list_collections', 'get_collection', 'list_attributes', 'build_cohort', 'get_cohort_urls', 'get_viewer_url', 'run_sql', 'idc_status'],
    needs: 'Nothing. IDC\'s data is public and the hosted server needs no key. Network access to api.imaging.datacommons.cancer.gov.',
    sources: ['NCI Imaging Data Commons REST API (api.imaging.datacommons.cancer.gov)']
  },
  literatureagent: {
    domain: 'Literature',
    what: 'Built on the BRXT adapter kit: every request goes to a fixed address, and a key is sent only to its own service. It reads scholarly metadata from OpenAlex, Crossref, PubMed (NCBI E-utilities), Semantic Scholar, bioRxiv and medRxiv, and Unpaywall, and returns the JSON with the exact request, so DOIs, PMIDs, titles and authors can be checked against the source. It is read only. It replaces StructuredPaperSearchAgent.',
    asks: [
      'Check the DOIs in this reference list against Crossref and flag any that do not resolve.',
      'Find OpenAlex works on multiple sclerosis and single cell RNA-seq from the last two years.',
      'Look up this PMID in PubMed and confirm the authors, journal and year.'
    ],
    tools: ['literature_status', 'literature_get'],
    needs: 'LITERATURE_CONTACT_EMAIL, which the services use to identify the caller (Unpaywall refuses requests without it). SEMANTIC_SCHOLAR_API_KEY is optional and goes only to api.semanticscholar.org. It needs uv.',
    sources: [
      'PubMed, through NCBI E-utilities',
      'OpenAlex API',
      'Crossref REST API',
      'Semantic Scholar Graph API v1',
      'bioRxiv and medRxiv API (api.biorxiv.org)',
      'Unpaywall API v2'
    ]
  },
  'folklore-clinical-variant-interpretation-mcp': {
    domain: 'Genomics & omics',
    what: 'Biorouter rebuild of the Folklore MCP 2.0.0 anonymous bridge to the hosted service from Helena Bioinformatics. It classifies one public GRCh38 germline SNV or simple indel under ACMG/AMP, retaining the applied criteria, source versions and uncertainty without choosing between ambiguous candidates. It also resolves genes, retrieves gene profiles and ClinGen gene-disease associations, finds disease-associated genes, and searches variant, gene and scientific literature. Results require qualified professional review and are not a diagnosis or treatment recommendation.',
    asks: [
      'What does NM_007294.4:c.68_69del mean under ACMG/AMP?',
      'Review the evidence for this VUS and show which criteria were applied.',
      'Resolve BRCA1, then retrieve its gene profile and ClinGen gene-disease associations.',
      'Find papers that mention this variant and give me the abstract of the most relevant one.'
    ],
    tools: ['classify_germline_variant', 'search_variant_literature', 'get_publication_details', 'search_literature_corpus', 'support_helena', 'get_gene_disease_associations', 'search_disease_genes', 'resolve_gene', 'get_gene_profile', 'get_gene_section', 'search_gene_literature'],
    needs: 'No account, API key or environment variable. Outbound HTTPS to https://api.helena.bio/folklore/v1/mcp. Send public variant, gene and disease identifiers and scientific literature questions only, with no patient, private case, phenotype, family or segregation details. A qualified professional must review the results.',
    sources: [
      'Folklore API by Helena Bioinformatics (api.helena.bio)',
      'The Folklore genetics literature corpus, drawn from PubMed',
      'ClinVar assertions and population frequency evidence, as Folklore reports them',
      'ClinGen Gene-Disease Validity, through the hosted service'
    ]
  },
  chemoinformaticsagent: {
    domain: 'Chemistry & structure',
    what: 'Computes molecular descriptors, fingerprint similarity, substructure matches and standardised structures on this computer with RDKit (BSD-3-Clause), and looks compounds up in PubChem and ChEMBL. It is the one BAAM extension that owns PubChem and ChEMBL. It runs no local commands.',
    asks: [
      'Look up caffeine in PubChem and give me its CID, canonical SMILES and molecular weight.',
      'Pull ChEMBL activity records for imatinib and summarize the targets.',
      'Write and run a Python script that computes RDKit descriptors for these SMILES.'
    ],
    tools: ['chem_descriptors', 'chem_similarity', 'chem_substructure', 'chem_standardize', 'chem_get', 'chem_status'],
    needs: 'No API keys, because PubChem and ChEMBL are public. RDKit installs with the extension. It needs uv.',
    sources: [
      'PubChem PUG REST (pubchem.ncbi.nlm.nih.gov/rest/pug)',
      'ChEMBL API (www.ebi.ac.uk/chembl/api/data)',
      'Local tools: python, uv, obabel, rdkit, datamol'
    ]
  },
  proteinstructureagent: {
    domain: 'Chemistry & structure',
    what: 'Built on the BRXT adapter kit: every request goes to a fixed address, and a key is sent only to its own service. It reads public structure and sequence records from the RCSB PDB data and search APIs, AlphaFold DB and UniProt. It is read only, and it runs no local programs.',
    asks: [
      'Find PDB entries for human KRAS and list their resolution and experimental method.',
      'Get the AlphaFold model metadata for UniProt P04637.',
      'Align these two structure files with US-align and report the TM-score.'
    ],
    tools: ['proteinstructure_status', 'proteinstructure_get'],
    needs: 'Nothing. No API keys. It needs uv.',
    sources: [
      'RCSB PDB Data API (data.rcsb.org/rest/v1)',
      'RCSB Search API (search.rcsb.org/rcsbsearch/v2)',
      'AlphaFold DB API (alphafold.ebi.ac.uk/api)',
      'UniProt REST (rest.uniprot.org)'
    ]
  },
  opentronsagent: {
    domain: 'Lab & ELN',
    what: 'Simulates protocols written with the Opentrons Python API through the opentrons 10.1.0 package (Apache-2.0) and returns the run log, step counts and any error, such as a pipette asked to hold more than its volume or labware in a slot that cannot take it. A simulation runs the protocol\'s Python on this computer, so the source must first pass a static check (only opentrons and a few standard modules may be imported; no eval, exec, open, str.format or underscore attributes), a run stops after 60 seconds or 2 million protocol lines, and Biorouter asks before each run. Opentrons 10 simulates Flex protocols only. It also searches and shows the standard labware definitions that ship with the package, with no network call, and reads Opentrons shared data on GitHub. It never talks to a robot.',
    asks: [
      'Simulate this Flex protocol and tell me which step fails, if any.',
      'Find the 96 well plates that ship with opentrons and show the well volume and depth of corning_96_wellplate_360ul_flat.',
      'List the pipette definitions in Opentrons shared data on GitHub.'
    ],
    tools: ['opentrons_simulate', 'opentrons_labware_search', 'opentrons_labware_definition', 'opentrons_status', 'opentrons_get'],
    skills: ['opentrons-lab-automation'],
    needs: 'Nothing. No account or API key. It needs uv and Python 3.12 or later; the opentrons package installs with the extension.',
    sources: [
      'The opentrons 10.1.0 simulator and its labware definitions, on this computer',
      'Opentrons shared data on GitHub (raw.githubusercontent.com/Opentrons/opentrons)',
      'GitHub contents API for the Opentrons repository (api.github.com)'
    ]
  },
  lamindbagent: {
    domain: 'Lab & ELN',
    what: 'Connects to the LaminDB instance named in LAMINDB_INSTANCE (owner/name) with the lamindb 2.12.2 library (Apache-2.0) and reads its registry: LaminDB\'s ranked text search, field filters over one registry, and one record\'s description with its features, labels, storage path and lineage. Every tool only reads; filter, field and sort names are checked against the registry\'s model before a query runs, and nothing is saved, deleted or moved. lamindb keeps its settings in the extension\'s own data folder, so your ~/.lamin is neither read nor changed, and your key is handed to lamindb inside the extension\'s own process. lamindb_get reaches a LaminHub API address only if you set LAMINDB_API_URL.',
    asks: [
      'Which LaminDB instance am I connected to, and how many artifacts and collections does it hold?',
      'Find the .h5ad artifacts whose key starts with scrna/ and list their keys and versions.',
      'Describe this artifact by its uid and show the run and transform that produced it.'
    ],
    tools: ['lamindb_instance', 'lamindb_search', 'lamindb_query', 'lamindb_describe', 'lamindb_status', 'lamindb_get'],
    skills: ['lamindb-data-registry'],
    needs: 'A LaminDB instance, named in LAMINDB_INSTANCE as owner/name. For a LaminHub instance, LAMINDB_API_KEY (secret), which goes only to Lamin. It needs uv; lamindb installs with the extension.',
    sources: ['Your LaminDB instance (its registry database and storage), through the lamindb library', 'LaminHub (lamin.ai)']
  },
  cellxgenecensusagent: {
    domain: 'Genomics & omics',
    what: 'Queries the CELLxGENE Census of single-cell data with the cellxgene-census library (MIT): Census releases, datasets, cell counts, cell metadata and per-group gene expression summaries, each with a size cap, plus the CELLxGENE Discover curation API. It replaces SingleCellOmicsAgent.',
    asks: [
      'List public CELLxGENE collections that include human brain data.',
      'Get the metadata for this CELLxGENE dataset ID and list its assays and tissues.',
      'Summarize this exported collection JSON and keep the dataset IDs.'
    ],
    tools: ['cellxgene_census_versions', 'cellxgene_datasets', 'cellxgene_cell_counts', 'cellxgene_cell_metadata', 'cellxgene_expression_summary', 'cellxgene_summary_counts', 'cellxgene_get', 'cellxgene_status'],
    needs: 'Nothing. No account or API key. The Census is read from public storage. It needs uv.',
    sources: ['CELLxGENE API (api.cellxgene.cziscience.com)']
  },
  primekgagent: {
    domain: 'Knowledge graphs',
    what: 'Downloads the PrimeKG precision medicine knowledge graph once from Harvard Dataverse (doi:10.7910/DVN/IXA7BM), checks its checksums, and answers queries on this computer: node lookup, neighbours and short paths between drugs, diseases, genes and phenotypes.',
    asks: [
      'Fetch the PrimeKG README from its GitHub repository and summarize what the graph contains.',
      'Use the GitHub API target to list the files in the mims-harvard/PrimeKG repository.',
      'Check the status of PrimeKGAgent and show which targets and environment variables are configured.'
    ],
    tools: ['primekg_download', 'primekg_data_status', 'primekg_find_nodes', 'primekg_neighbours', 'primekg_paths', 'primekg_get', 'primekg_status'],
    needs: 'No credentials. About 1 GB of disk and network access once, for the download. It needs uv.',
    sources: ['PrimeKG repository files (raw.githubusercontent.com/mims-harvard/PrimeKG)', 'GitHub REST API (api.github.com)']
  },
  zoteroagent: {
    domain: 'Literature',
    what: 'Built on the BRXT adapter kit: every request goes to a fixed address, and a key is sent only to its own service. It reaches the Zotero Web API for a user or group library: items, collections, tags and searches. Public libraries read without a key. zotero_get only reads, so it runs without asking; zotero_write only creates items and collections, and Biorouter asks before each call; editing and deleting are not offered.',
    asks: [
      'List the collections in my Zotero user library (my user ID is 1234567).',
      'Find items tagged multiple sclerosis in our group library and return their titles and DOIs.',
      'Summarize this exported Zotero item JSON and show its identifiers.'
    ],
    tools: ['zotero_status', 'zotero_get', 'zotero_write'],
    needs: 'ZOTERO_API_KEY (secret) for a private library, sent only to api.zotero.org. ZOTERO_USER_ID or ZOTERO_GROUP_ID names the library. It needs uv.',
    sources: ['Zotero Web API (api.zotero.org)']
  },
  markitdownagent: {
    domain: 'Productivity',
    what: 'Wraps Microsoft\'s MarkItDown 0.1.9 (MIT) as one tool that converts a file or URL to Markdown. It is meant for the formats the built-in document tools do not read: PowerPoint, EPUB, Outlook .msg, older .xls, notebooks, ZIP archives, image and audio metadata, YouTube transcripts, RSS and Wikipedia pages.',
    asks: [
      'Check whether the markitdown command is installed.',
      'Convert report.pdf in ~/Documents to Markdown with markitdown.',
      'Plan the markitdown command to convert slides.pptx and show me what will run before running it.'
    ],
    tools: ['convert_to_markdown'],
    needs: 'No credentials. MarkItDown and its converters install with the extension. It needs uv.',
    sources: ['Local documents you name']
  },
  adaptyvbiofoundryagent: {
    domain: 'Lab & ELN',
    what: 'Built on the BRXT adapter kit: every request goes to a fixed address, and a key is sent only to its own service. It reaches Adaptyv Bio\'s foundry API: experiments, cost estimates and results. Submitting or confirming an experiment can spend money, so its skill estimates first; adaptyv_write changes data, so Biorouter asks you before each call. Paying invoices and managing tokens or webhooks are not offered.',
    asks: [
      'Check the status of AdaptyvBioFoundryAgent and confirm whether an API key is configured.',
      'Fetch the status and result metadata for my Adaptyv experiment and keep the job IDs.',
      'Draft the request to submit this sequence library to Adaptyv, but do not send it.'
    ],
    tools: ['adaptyv_status', 'adaptyv_get', 'adaptyv_write'],
    needs: 'ADAPTYV_API_KEY (secret), sent only to devs.adaptyvbio.com. It needs uv.',
    sources: ['Adaptyv Bio API (api.adaptyvbio.com)']
  },
  scientificwebresearchagent: {
    domain: 'Web & browser',
    what: 'Built on the BRXT adapter kit: every request goes to a fixed address, and a key is sent only to its own service. It searches the web with Exa and Tavily through two typed tools, and webresearch_get reaches their other read routes. Each key is sent only to its own service. It is general web search, not scholarly search: LiteratureAgent is the extension for papers.',
    asks: [
      'Search the web with Tavily for recent guidance on CAR-T neurotoxicity and list the source URLs.',
      'Use Exa to find lab protocols for organoid culture outside journal databases.',
      'Fetch this conference page and summarize it, keeping the URL for citation.'
    ],
    tools: ['exa_search', 'tavily_search', 'webresearch_get', 'webresearch_status'],
    needs: 'EXA_API_KEY, TAVILY_API_KEY or both (secret). A search needs the key of the service it uses. It needs uv.',
    sources: ['Exa API (api.exa.ai)', 'Tavily API (api.tavily.com)', 'Any URL you give it']
  },
  tamarindbioagent: {
    domain: 'Chemistry & structure',
    what: 'Built on the BRXT adapter kit: every request goes to a fixed address, and a key is sent only to its own service. It reaches the Tamarind Bio API for hosted folding, design and docking jobs. Submitting a job uses your Tamarind quota, so its skill validates and estimates first; tamarind_write changes data, so Biorouter asks you before each call.',
    asks: [
      'Check whether TamarindBioAgent has an API key set and which base URL it will call.',
      'List my Tamarind jobs with the name and status of each.',
      'Draft the request to submit this docking job to Tamarind, and wait for my approval before sending it.'
    ],
    tools: ['tamarind_status', 'tamarind_get', 'tamarind_write'],
    needs: 'TAMARIND_API_KEY (secret) from your Tamarind Bio account, sent only to app.tamarind.bio. It needs uv.',
    sources: ['Tamarind Bio API (api.tamarind.bio)']
  },
  rowanmolecularmodelingagent: {
    domain: 'Chemistry & structure',
    what: 'Submits Rowan cloud molecular modelling workflows for one molecule (a SMILES), polls their status and reads their results, through rowan-python 3.3.4 (MIT). rowan_workflow_types lists the workflow types it can submit, such as pKa, conformer search, descriptors, solubility and docking, with no network call. A submission spends credits from your Rowan account, so rowan_submit_workflow is marked destructive and Biorouter asks before every call; it takes max_credits to cap the cost, and a draft is saved in Rowan without starting and costs nothing. rowan-python\'s address is pinned to api.rowansci.com, so a ROWAN_API_URL in the environment cannot move the key, and the key is removed from every error. rowan_get and rowan_write reach the same API route by route, and Biorouter asks before each rowan_write call.',
    asks: [
      'List the Rowan workflow types this extension can submit.',
      'Save a pKa workflow for this SMILES as a draft in Rowan so I can check its parameters first.',
      'Check the status of these Rowan workflow ids and read the results of the completed ones.'
    ],
    tools: ['rowan_workflow_types', 'rowan_submit_workflow', 'rowan_workflow_status', 'rowan_workflow_results', 'rowan_status', 'rowan_get', 'rowan_write'],
    skills: ['rowan-molecular-modeling'],
    needs: 'ROWAN_API_KEY (secret) from your Rowan account, sent only to api.rowansci.com. A submission spends your Rowan credits. It needs uv and Python 3.13 or later; rowan-python and RDKit install with the extension.',
    sources: ['Rowan API (api.rowansci.com), through rowan-python']
  },
  gptragent: {
    domain: 'Code',
    what: 'Connects Biorouter to an R session you already have open, through the gptr package\'s MCP endpoint. The agent can find your running sessions, look at objects in memory without reloading data, and run R code in that session to analyse, plot or fix things in place. Every call passes gptr\'s own permission settings, and Biorouter asks before code runs. The extension is private: a chat on a public model never sees it, because a live session holds whatever you loaded. Made by the Baranzini Lab (Wanjun Gu, UCSF); the bridge lives in the gptr repository and is released with gptr.',
    asks: [
      'Look at the data frame I have loaded and tell me what is wrong with the dates.',
      'Fit a mixed model on df in my session and plot the residuals.',
      'My last command failed; read the traceback and fix it in place.'
    ],
    tools: ['sessions', 'inspect', 'r'],
    needs: 'R with the gptr package (1.0 or later; plan mode needs 1.2.0 or later, and recording 1.3.0), a session served through gptr\'s MCP endpoint, and uv. Served with no mode named, gptr 1.3.0 and later use auto mode by default, so the agent may change objects; mode = "manual" lets it only read. A private model in Biorouter.',
    sources: [
      'Your own R session, through gptr (Broccolito/gptr)'
    ]
  },
  biomcpagent: {
    domain: 'Genomics & omics',
    what: 'Launches BioMCP 0.9.3 (genomoncology/biomcp, MIT) for entity-centred biomedical search: start from a gene, variant, drug, disease or trial and pivot to the others across curated sources. Each data source\'s own terms apply, and some restrict commercial or clinical use. It replaces ClinicalVariantAgent.',
    asks: [
      'What is known about BRAF V600E, and which trials are recruiting for it?',
      'Summarise ClinVar and MyVariant evidence for this rsID.',
      'Find recent articles that mention this variant together with resistance.'
    ],
    tools: ['search', 'get', 'variant_articles', 'variant_normalize_car', 'variant_erepo', 'gene_cspec', 'biomcp'],
    needs: 'No credentials for the public sources. Some sources, such as OncoKB, give more with their own key. Network access. It needs uv.',
    sources: ['PubMed, Europe PMC and PubTator', 'ClinVar and MyVariant.info', 'MyGene.info', 'ClinicalTrials.gov', 'OncoKB', 'OpenFDA', 'Reactome', 'cBioPortal']
  },
  opencomputeruseagent: {
    domain: 'Productivity',
    what: 'A fallback for desktop control where the built-in Biorouter Copilot does not work, built on Open Computer Use 1.1.5 (MIT) with patches. Use it on Linux with at-spi2-core older than 2.52 (Ubuntu 22.04, Debian 12, Rocky 9), or for an app whose controls Copilot cannot read. It asks once per chat before it acts, refuses what Copilot refuses, never types into password fields or terminals, and refuses whole-display capture.',
    asks: [
      'List the apps that are running and show me the state of the spreadsheet window.',
      'In this app, open the Export dialog and set the format to CSV.',
      'Check whether this computer can run the desktop control fallback.'
    ],
    tools: ['list_apps', 'get_app_state', 'screen_capture', 'click', 'type_text', 'press_key', 'set_value', 'setup_status'],
    needs: 'Biorouter 1.94.6 or later and uv. macOS 14 or later (the helper app holds the Accessibility and Screen Recording permissions), Windows 10 or 11 on x64, or Linux on X11 with the distro\'s Python, PyGObject, AT-SPI and GTK 3 packages.',
    sources: [
      'Your desktop\'s accessibility tree, through the bundled Open Computer Use helper'
    ]
  },
  unifiedpublicdatabaselookupagent: {
    domain: 'Genomics & omics',
    what: 'Built on the BRXT adapter kit: every request goes to a fixed address, and a key is sent only to its own service. Public Bio Databases looks up genes, sequences, proteins, variants and drug-gene annotations in NCBI E-utilities and BLAST, UniProt, Ensembl and ClinPGx. It is read only. Literature search moved to LiteratureAgent and chemistry to ChemoinformaticsAgent. It replaces NCBIAgent.',
    asks: [
      'Look up UniProt entry P04637 and give me the accession and source URL.',
      'Find the PubChem CID and the ChEMBL ID for imatinib.',
      'Get the Crossref metadata for DOI 10.1038/nature12373.'
    ],
    tools: ['publicdb_status', 'publicdb_get'],
    needs: 'Nothing. No API keys. PUBLIC_DB_CONTACT_EMAIL is optional and identifies the caller to NCBI. It needs uv.',
    sources: [
      'NCBI E-utilities', 'UniProt REST', 'Ensembl REST', 'OpenAlex', 'Crossref', 'ChEMBL', 'PubChem PUG REST', 'SEC data.sec.gov'
    ]
  }
};
