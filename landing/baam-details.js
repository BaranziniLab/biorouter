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
    tools: ['build_cohort', 'query', 'search_note_concepts', 'search_diagnoses_by_code', 'crossmap_patient', 'get_database_overview'],
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
    tools: ['query_ucsf_omop', 'search_concepts', 'find_measurement', 'get_omop_schema', 'list_ucsf_omop_tables'],
    needs: 'UCSF network or VPN access, and read permission on OMOP_DEID. Your UCSF network login as CLINICAL_RECORDS_USERNAME and CLINICAL_RECORDS_PASSWORD. uv and Python 3.11 or later.',
    sources: [
      'UCSF OMOP_DEID (OMOP CDM v5.4 on Microsoft SQL Server, schema omop)',
      'OMOP standard vocabularies (the concept tables)'
    ]
  },
  playwrightagent: {
    domain: 'Web & browser',
    what: 'Packages Microsoft\'s @playwright/mcp server as a Biorouter extension, started by a small Python launcher through npx. The agent can open pages, click, type, fill forms, handle dialogs, manage tabs and read the structure of a page through accessibility snapshots, so no vision model is needed. The launcher runs the latest @playwright/mcp release with its default options, so the tool list follows that release.',
    asks: [
      'Open arxiv.org and find papers on retrieval augmented generation from the last week.',
      'Search ClinicalTrials.gov for multiple sclerosis remyelination and summarize the first ten active trials.',
      'Fill out the contact form on example.com with my name and email.'
    ],
    tools: ['browser_navigate', 'browser_snapshot', 'browser_click', 'browser_type', 'browser_fill_form', 'browser_tabs'],
    needs: 'Node.js 18 or later with npx on PATH, and access to the npm registry when it starts. Biorouter v1.20.0 or later. No credentials.',
    sources: ['Any website the browser visits', 'The @playwright/mcp package from npm (Microsoft)']
  },
  spokeagent: {
    domain: 'Knowledge graphs',
    what: 'Queries SPOKE, a Neo4j biomedical knowledge graph of about 43 million nodes that links diseases, genes, proteins, compounds, pathways, anatomy and side effects. It reads the live schema into a compact map of edge types, resolves names, synonyms and identifiers to canonical nodes, describes the relationships of a node, and finds the shortest paths between two entities. Its Cypher queries can only read, with an automatic LIMIT and a time limit on each transaction.',
    asks: [
      'Which genes are associated with Alzheimer\'s disease?',
      'How are metformin and multiple sclerosis connected?',
      'What relationship types does the TNF gene node have, and how many of each?'
    ],
    tools: ['get_spoke_schema', 'resolve_entity', 'describe_node', 'find_path', 'query_spoke'],
    skills: ['spoke-knowledge-graph'],
    needs: 'SPOKEAGENT_PASSCODE, from the credentials page below. uv and Python 3.11 or later.',
    sources: [
      'SPOKE knowledge graph (Neo4j, about 43 million nodes)',
      'Identifiers it resolves: DOID, Entrez, Ensembl, DrugBank, UMLS CUI, UBERON and GO'
    ],
    links: [{ label: 'SPOKEAgent credentials page', href: 'https://wiki.library.ucsf.edu/spaces/~Wanjun.Gu@ucsf.edu/pages/755904655/SPOKEAgent+Credentials' }]
  },
  codegraphagent: {
    domain: 'Code',
    what: 'Indexes a local project into a SQLite graph of symbols and call edges, so the agent can find callers, callees, call paths and the effect of a change without searching the files each time. It is built on CodeGraph and tree-sitter, and adds R, Julia, MATLAB and Perl to the 19 languages CodeGraph reads, 23 in all. On first use the agent calls codegraphagent_check_engine, which downloads the engine once per machine (about 45 MB), then codegraphagent_index_project before queries return results.',
    asks: [
      'Who calls normalize_counts in this R package?',
      'What breaks if I change the signature of load_config?',
      'Show the call path from main to write_results.'
    ],
    tools: ['codegraphagent_check_engine', 'codegraphagent_index_project', 'codegraph_callers', 'codegraph_callees', 'codegraph_impact', 'codegraph_search'],
    needs: 'No credentials. Network access once, to download the engine (about 45 MB) from the repository\'s GitHub releases. On a machine without network access, set CODEGRAPH_ENGINE_PATH to an extracted copy. It needs uv. The engine carries its own Node.js.',
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
    needs: 'No credentials and no Microsoft Office. OfficeCLI v1.0.108 ships inside the bundle for macOS on Apple silicon. Other platforms download the same release the first time it starts. It needs uv.',
    sources: ['Local .docx, .xlsx and .pptx files']
  },
  benchlingagent: {
    domain: 'Lab & ELN',
    what: 'Connects to the REST API of your Benchling tenant (https://<tenant>.benchling.com/api/v2) with an API key. It has one guarded tool for API calls rather than a tool for each resource. That tool sends GET requests to endpoints such as /projects, /registry-entities and /entries, and refuses POST, PUT, PATCH and DELETE unless allow_mutation=true is passed after you approve. Helper tools report the configuration without showing secrets, say whether a request would read or change data, and summarize a JSON payload without a network call.',
    asks: [
      'List the Benchling projects I can see.',
      'Look up the registry entities in this project and list their IDs and owners.',
      'Summarize this notebook entry export and keep its IDs and timestamps.'
    ],
    tools: ['call_benchlingagent_api', 'get_benchlingagent_request_plan', 'get_benchlingagent_status', 'summarize_benchlingagent_resource'],
    needs: 'BENCHLING_API_KEY (secret) and BENCHLING_TENANT (the subdomain, for example mylab for https://mylab.benchling.com). A Benchling account whose key can reach the data. It needs uv.',
    sources: ['Benchling REST API v2 on your tenant', 'Benchling JSON exports you hand to summarize_benchlingagent_resource']
  },
  dnanexusagent: {
    domain: 'Compute & workflows',
    what: 'Sends requests to the DNAnexus platform API (https://api.dnanexus.com by default) with your API token. It reports whether the token is set, checks whether a request would change anything before it is sent, makes the call, and summarizes a returned JSON record. POST, PUT, PATCH and DELETE are refused unless allow_mutation=true is passed after you approve the exact operation.',
    asks: [
      'Is my DNAnexus token configured, and which API server is this extension using?',
      'Describe project-xxxx and list its owner, creation date and version fields.',
      'Before running anything, tell me whether this DNAnexus job launch would change anything.'
    ],
    tools: ['get_dnanexusagent_status', 'get_dnanexusagent_request_plan', 'call_dnanexusagent_api', 'summarize_dnanexusagent_resource'],
    skills: ['dnanexus-cloud-analysis'],
    needs: 'DNANEXUS_API_TOKEN (required, secret, sent as a Bearer header). Optional DNANEXUS_API_SERVER (https://api.dnanexus.com by default) and DNANEXUS_LOG_LEVEL (INFO by default). Python 3.11 or later, from the .brxt bundle.',
    sources: ['DNAnexus platform API (api.dnanexus.com)']
  },
  latchbioagent: {
    domain: 'Compute & workflows',
    what: 'Sends requests to the LatchBio API (https://api.latch.bio by default) with your LatchBio token. It reports whether the token is set, checks whether a request would change anything, makes the call, and summarizes a returned JSON record. Write methods (POST, PUT, PATCH, DELETE), including workflow launches, are refused unless allow_mutation=true is passed after you approve.',
    asks: [
      'Check that my LatchBio token is configured.',
      'List the workflows in my LatchBio workspace with their IDs and versions.',
      'Summarize this execution record and pull out its ID, owner and timestamps.'
    ],
    tools: ['get_latchbioagent_status', 'get_latchbioagent_request_plan', 'call_latchbioagent_api', 'summarize_latchbioagent_resource'],
    skills: ['latchbio-workflows'],
    needs: 'LATCHBIO_API_TOKEN (required, secret, sent as a Bearer header). Optional LATCHBIO_BASE_URL (https://api.latch.bio by default) and LATCHBIO_LOG_LEVEL (INFO by default). Python 3.11 or later, from the .brxt bundle.',
    sources: ['LatchBio API (api.latch.bio)']
  },
  labarchivesagent: {
    domain: 'Lab & ELN',
    what: 'Sends requests to the LabArchives API (https://api.labarchives.com by default) with your API key. It reports whether the key is set, checks whether a request would change anything, makes the call, and summarizes a returned JSON record with its IDs, authors and timestamps in view. Notebook edits and any other POST, PUT, PATCH or DELETE request are refused unless allow_mutation=true is passed after you approve.',
    asks: [
      'Is my LabArchives API key configured?',
      'List my LabArchives notebooks and their folders.',
      'Summarize this notebook entry export and show its author and when it was last changed.'
    ],
    tools: ['get_labarchivesagent_status', 'get_labarchivesagent_request_plan', 'call_labarchivesagent_api', 'summarize_labarchivesagent_resource'],
    skills: ['labarchives-eln'],
    needs: 'LABARCHIVES_API_KEY (required, secret, sent as a Bearer header). Optional LABARCHIVES_BASE_URL (https://api.labarchives.com by default) and LABARCHIVES_LOG_LEVEL (INFO by default). Python 3.11 or later, from the .brxt bundle.',
    sources: ['LabArchives API (api.labarchives.com)']
  },
  omeroagent: {
    domain: 'Imaging',
    what: 'Sends requests to the OMERO server you name in OMERO_BASE_URL, such as its /api/ and /webclient/api/ JSON endpoints, to read project, dataset, image and annotation metadata. It reports the configured settings, checks whether a request would change anything, makes the call, and summarizes a returned JSON record. Write methods are refused unless allow_mutation=true is passed after you approve. The bundled skill keeps downloads narrow and avoids exporting images that may carry protected health information.',
    asks: [
      'Which OMERO server is configured, and does it answer at /api/?',
      'List the datasets on our OMERO server.',
      'Summarize the metadata for this image record and keep its ID, owner and URL.'
    ],
    tools: ['get_omeroagent_status', 'get_omeroagent_request_plan', 'call_omeroagent_api', 'summarize_omeroagent_resource'],
    skills: ['omero-imaging-data'],
    needs: 'OMERO_BASE_URL (required; the default in the code is a placeholder, https://omero.example.org). Optional OMERO_USERNAME and OMERO_PASSWORD (secret), and OMERO_LOG_LEVEL (INFO by default). Python 3.11 or later, from the .brxt bundle.',
    sources: ['The JSON API of your OMERO server']
  },
  protocolsioagent: {
    domain: 'Lab & ELN',
    what: 'Sends requests to the Protocols.io API v3 (https://www.protocols.io/api/v3 by default) to find and read protocol records, and keeps their IDs, versions, authors, DOIs and URLs for citation. A token is optional. Without one it makes anonymous requests, and with PROTOCOLS_IO_TOKEN set it can reach private workspaces. Write methods are refused unless allow_mutation=true is passed after you approve.',
    asks: [
      'Search Protocols.io for nuclei isolation protocols for single nucleus RNA-seq.',
      'Fetch protocol 12345 and give me its title, version, authors and DOI for citation.',
      'Summarize this protocol record and list its provenance fields.'
    ],
    tools: ['get_protocolsioagent_status', 'get_protocolsioagent_request_plan', 'call_protocolsioagent_api', 'summarize_protocolsioagent_resource'],
    skills: ['protocols-io-methods'],
    needs: 'Nothing required. Optional PROTOCOLS_IO_TOKEN (secret, sent as a Bearer header when set) for private workspaces, PROTOCOLS_IO_BASE_URL (https://www.protocols.io/api/v3 by default) and PROTOCOLS_IO_LOG_LEVEL (INFO by default). Python 3.11 or later, from the .brxt bundle.',
    sources: ['Protocols.io API v3']
  },
  opennotebookagent: {
    domain: 'Lab & ELN',
    what: 'Sends requests to a notebook server you host and name in OPEN_NOTEBOOK_BASE_URL, for example its /entries, /projects or /search endpoints. It reads entry timestamps, authors and attachments without overwriting records. Its summarize tool checks provenance: it lists which of id, name, created_at, updated_at, owner, author, version, doi and url a returned record carries, without a network call. Write methods are refused unless allow_mutation=true is passed after you approve.',
    asks: [
      'Which notebook server is configured, and is a token set?',
      'Search our lab notebook for entries about the March CRISPR screen.',
      'Check the provenance of this entry: who wrote it, when, and which version?'
    ],
    tools: ['get_opennotebookagent_status', 'get_opennotebookagent_request_plan', 'call_opennotebookagent_api', 'summarize_opennotebookagent_resource'],
    skills: ['open-notebook-research'],
    needs: 'OPEN_NOTEBOOK_BASE_URL (required; the default in the code is a placeholder, https://notebook.example.org/api). Optional OPEN_NOTEBOOK_TOKEN (secret, sent as a Bearer header when set) and OPEN_NOTEBOOK_LOG_LEVEL (INFO by default). Your own notebook server with a JSON HTTP API. Python 3.11 or later, from the .brxt bundle.',
    sources: ['The JSON API of your notebook server']
  },
  modalagent: {
    domain: 'Compute & workflows',
    what: 'Reports whether the Modal token ID and secret are set, and sends requests to a Modal HTTP endpoint (https://api.modal.com by default) with an Authorization: Token id:secret header. It checks whether a request would change anything before it is sent, and summarizes a returned JSON record. Deploys, job runs and any other POST, PUT, PATCH or DELETE request are refused unless allow_mutation=true is passed after you approve.',
    asks: [
      'Are my Modal token ID and secret both configured?',
      'Before deploying anything, tell me whether this Modal request would change anything.',
      'Summarize this Modal app record and keep its ID, version and timestamps.'
    ],
    tools: ['get_modalagent_status', 'get_modalagent_request_plan', 'call_modalagent_api', 'summarize_modalagent_resource'],
    skills: ['modal-compute'],
    needs: 'MODAL_TOKEN_ID and MODAL_TOKEN_SECRET (both required, both secret). Optional MODAL_LOG_LEVEL (INFO by default). The server also reads MODAL_API_BASE_URL (https://api.modal.com by default), which its manifest does not list. Python 3.11 or later, from the .brxt bundle. The Modal CLI is not used.',
    sources: ['Modal HTTP API (api.modal.com)']
  },
  idcagent: {
    domain: 'Imaging',
    what: 'Calls the REST API of the NCI Imaging Data Commons (IDC) and returns the JSON response, so you can look through public cancer imaging collections, analysis results and cohort metadata before you plan a download. GET requests run freely. POST, PUT, PATCH and DELETE are refused unless the call passes allow_mutation=true after you approve. Two more tools preview the URL and approval needs of a request, or summarize a saved JSON export offline.',
    asks: [
      'Which IDC collections cover lung cancer, and how many subjects does each one have?',
      'Show the metadata IDC holds for the 4D-Lung collection.',
      'Summarize this IDC cohort export and list the IDs, versions and URLs it contains.'
    ],
    tools: ['get_idcagent_status', 'get_idcagent_request_plan', 'call_idcagent_api', 'summarize_idcagent_resource'],
    needs: 'No account for public metadata. IDC_API_TOKEN is optional and is sent as a bearer token. Set IDC_API_BASE_URL to https://api.imaging.datacommons.cancer.gov/v3, because the v1 default in the code now returns HTTP 410.',
    sources: ['NCI Imaging Data Commons REST API (api.imaging.datacommons.cancer.gov)']
  },
  literatureagent: {
    domain: 'Literature',
    what: 'Sends HTTP requests to six literature APIs (PubMed E-utilities, OpenAlex, Crossref, Semantic Scholar, bioRxiv and medRxiv, and Unpaywall) and returns the raw JSON with the exact request URL, so DOIs, PMIDs, titles and authors can be checked against the source. HTTP methods that write are refused unless the call passes allow_mutation=true after you approve. A summarize tool lists the provenance fields in a saved payload (DOI, PMID, PMCID, version, dates) offline.',
    asks: [
      'Check the DOIs in this reference list against Crossref and flag any that do not resolve.',
      'Find OpenAlex works on multiple sclerosis and single cell RNA-seq from the last two years.',
      'Look up this PMID in PubMed and confirm the authors, journal and year.'
    ],
    tools: ['get_literatureagent_status', 'list_literatureagent_api_targets', 'call_literatureagent_api', 'summarize_literatureagent_resource'],
    needs: 'Nothing required. SEMANTIC_SCHOLAR_API_KEY is optional and is sent as x-api-key, since Semantic Scholar limits the rate of calls made without a key. Unpaywall needs an email query parameter on every call.',
    sources: [
      'PubMed, through NCBI E-utilities',
      'OpenAlex API',
      'Crossref REST API',
      'Semantic Scholar Graph API v1',
      'bioRxiv and medRxiv API (api.biorxiv.org)',
      'Unpaywall API v2'
    ]
  },
  ncbiagent: {
    domain: 'Genomics & omics',
    what: 'Calls NCBI E-utilities endpoints such as esearch, esummary, efetch and elink, plus the BLAST URL API, and returns the response with the exact request URL, so accessions and queries can be cited. Any Entrez database is reachable this way, including PubMed, Gene, SRA, GEO, ClinVar, dbSNP and Nucleotide. HTTP methods that write need allow_mutation=true after you approve, and a summarize tool pulls accession and provenance fields out of a saved payload offline.',
    asks: [
      'Find GEO series on multiple sclerosis in human blood and list their accessions.',
      'Get the Gene record for HLA-DRB1 and link it to its ClinVar entries.',
      'How many SRA runs belong to this BioProject accession?'
    ],
    tools: ['get_ncbiagent_status', 'list_ncbiagent_api_targets', 'call_ncbiagent_api', 'summarize_ncbiagent_resource'],
    needs: 'Nothing required. NCBI_EMAIL and NCBI_API_KEY can be set, but v0.1.0 only reports them in its status. The key is sent as an Authorization header rather than the api_key parameter that E-utilities reads, so expect the default limit of 3 requests per second.',
    sources: [
      'NCBI E-utilities (every Entrez database, for example PubMed, Gene, SRA, GEO, ClinVar, dbSNP, Nucleotide and Protein)',
      'NCBI BLAST URL API (Blast.cgi)'
    ]
  },
  'folklore-clinical-variant-interpretation-mcp': {
    domain: 'Genomics & omics',
    what: 'Connects to Folklore, a hosted service from Helena Bioinformatics that it can only read from. You give it one public GRCh38 germline SNV or simple indel, as HGVS, an rsID or an allele. It returns the resolved variant, an automated ACMG/AMP classification with the criteria it applied, source versions and the uncertainty it states, and it never chooses between ambiguous candidates for you. It also finds papers linked to the variant, returns full bibliographic records for the PMIDs it finds, and searches its literature corpus in plain language.',
    asks: [
      'What does NM_007294.4:c.68_69del mean under ACMG/AMP?',
      'Review the evidence for this VUS and show which criteria were applied.',
      'Find papers that mention this variant and give me the abstract of the most relevant one.'
    ],
    tools: ['search_variant_evidence', 'search_variant_literature', 'get_publication_details', 'search_literature_corpus'],
    needs: 'No account, API key or environment variable. Outbound HTTPS to https://api.helena.bio/folklore/v1/mcp. Send it public variant data only, with no patient, phenotype, family or segregation details. A qualified professional must review the results.',
    sources: [
      'Folklore API by Helena Bioinformatics (api.helena.bio)',
      'The Folklore genetics literature corpus, drawn from PubMed',
      'ClinVar assertions and population frequency evidence, as Folklore reports them',
      'ClinGen Gene-Disease Validity, through the hosted service'
    ]
  },
  clinicalvariantagent: {
    domain: 'Genomics & omics',
    what: 'Sends requests to NCBI E-utilities (for ClinVar and dbSNP records) and to MyVariant.info, and returns the raw JSON with the request URL, so identifiers and review status can be reported with their source. gnomAD allele frequencies come back as fields inside MyVariant.info records. Other resources, such as Ensembl VEP, have no preset target and need a full URL. It does not classify variants or give medical advice, and HTTP methods that write need allow_mutation=true after you approve.',
    asks: [
      'Get the ClinVar records for rs80357914 and report their review status.',
      'Look up chr7:g.140453136A>T in MyVariant.info and list its gnomAD exome allele frequencies.',
      'Summarize this ClinVar export and list the accession IDs and versions it contains.'
    ],
    tools: ['get_clinicalvariantagent_status', 'list_clinicalvariantagent_api_targets', 'call_clinicalvariantagent_api', 'summarize_clinicalvariantagent_resource'],
    needs: 'Nothing required. CLINICAL_VARIANT_CONTACT_EMAIL is optional and only appears in the status output. Results are evidence to review, not a diagnosis.',
    sources: ['ClinVar and dbSNP, through NCBI E-utilities', 'MyVariant.info v1 (with ClinVar, dbSNP and gnomAD fields)']
  },
  workflowrunneragent: {
    domain: 'Compute & workflows',
    what: 'Runs a fixed list of local commands (nextflow, nf-core, snakemake, cwltool, miniwdl, docker, singularity, apptainer, git, uv and python) without a shell, and only when the call carries allow_execute=true after you approve the exact command. That covers version checks, dry runs such as snakemake -n, validation such as cwltool --validate or miniwdl check, and full runs. It returns stdout, stderr and the exit code. It can also search the nf-core site and the Dockstore API for published pipelines.',
    asks: [
      'Which workflow engines are installed here, and which versions?',
      'Run this Snakefile with snakemake -n and tell me which rules would run.',
      'List the nf-core pipelines for RNA-seq and their latest releases.'
    ],
    tools: ['get_workflowrunneragent_status', 'list_workflowrunneragent_api_targets', 'call_workflowrunneragent_api', 'plan_workflowrunneragent_command', 'run_workflowrunneragent_command', 'summarize_workflowrunneragent_resource'],
    needs: 'No credentials. The engines you want must already be installed on the machine that runs Biorouter, and the status tool reports which ones are on PATH. Each command runs only after you approve it, with a time limit of up to 3600 seconds.',
    sources: ['nf-core website (nf-co.re)', 'Dockstore API (dockstore.org/api)', 'Workflow engines and container runtimes installed on your machine']
  },
  ucsfhpcagent: {
    domain: 'Compute & workflows',
    what: 'Runs a fixed list of commands (ssh, scp, rsync, sbatch, squeue, sacct, sinfo and scancel) without a shell, only when the call carries allow_execute=true after you approve the exact command, and returns stdout, stderr and the exit code. The commands run on the machine that hosts Biorouter, so SLURM commands reach the cluster either on a login node or through ssh. A plan tool checks a command against the list first, and a summarize tool reads a saved JSON payload offline.',
    asks: [
      'Show my queued and running jobs on the cluster with squeue.',
      'Use sacct to check how long job 123456 ran and how much memory it used.',
      'Copy results/ from the cluster to this laptop with rsync, after showing me the exact command.'
    ],
    tools: ['get_ucsfhpcagent_status', 'plan_ucsfhpcagent_command', 'run_ucsfhpcagent_command', 'summarize_ucsfhpcagent_resource'],
    needs: 'A UCSF HPC account and SSH access that works without an interactive password prompt, for example a key and a host alias. UCSF_HPC_HOST is optional and only shown in the status, so the host goes in each command.',
    sources: ['The UCSF HPC cluster, through SSH, scp, rsync and SLURM commands']
  },
  chemoinformaticsagent: {
    domain: 'Chemistry & structure',
    what: 'Makes HTTP calls to the PubChem PUG REST and ChEMBL APIs. Reads are the default, and POST, PUT, PATCH and DELETE are blocked unless allow_mutation=true. It can also run a short list of local commands (python, uv, obabel, rdkit, datamol) without a shell, only after you approve with allow_execute=true. That is how descriptor or SMILES cleanup scripts run: the extension has no chemistry functions of its own and does not bundle RDKit.',
    asks: [
      'Look up caffeine in PubChem and give me its CID, canonical SMILES and molecular weight.',
      'Pull ChEMBL activity records for imatinib and summarize the targets.',
      'Write and run a Python script that computes RDKit descriptors for these SMILES.'
    ],
    tools: ['get_chemoinformaticsagent_status', 'list_chemoinformaticsagent_api_targets', 'call_chemoinformaticsagent_api', 'plan_chemoinformaticsagent_command', 'run_chemoinformaticsagent_command', 'summarize_chemoinformaticsagent_resource'],
    needs: 'No API keys, because PubChem and ChEMBL are public. Local commands work only if the program is already on PATH (for example RDKit or Open Babel, installed separately). Python 3.11 or later, from the .brxt bundle.',
    sources: [
      'PubChem PUG REST (pubchem.ncbi.nlm.nih.gov/rest/pug)',
      'ChEMBL API (www.ebi.ac.uk/chembl/api/data)',
      'Local tools: python, uv, obabel, rdkit, datamol'
    ]
  },
  proteinstructureagent: {
    domain: 'Chemistry & structure',
    what: 'Calls four public APIs: RCSB PDB data, RCSB search, AlphaFold DB and UniProt REST. Write methods are blocked unless allow_mutation=true. It can also run a list of local structure programs (python, foldseek, mmseqs, pymol, phenix, usalign) without a shell, only after you approve with allow_execute=true. Those programs are not bundled.',
    asks: [
      'Find PDB entries for human KRAS and list their resolution and experimental method.',
      'Get the AlphaFold model metadata for UniProt P04637.',
      'Align these two structure files with US-align and report the TM-score.'
    ],
    tools: ['get_proteinstructureagent_status', 'list_proteinstructureagent_api_targets', 'call_proteinstructureagent_api', 'plan_proteinstructureagent_command', 'run_proteinstructureagent_command', 'summarize_proteinstructureagent_resource'],
    needs: 'No API keys. Local tools such as Foldseek, MMseqs2, PyMOL, Phenix or US-align must be installed separately and on PATH. Python 3.11 or later, from the .brxt bundle.',
    sources: [
      'RCSB PDB Data API (data.rcsb.org/rest/v1)',
      'RCSB Search API (search.rcsb.org/rcsbsearch/v2)',
      'AlphaFold DB API (alphafold.ebi.ac.uk/api)',
      'UniProt REST (rest.uniprot.org)'
    ]
  },
  singlecellomicsagent: {
    domain: 'Genomics & omics',
    what: 'Offers guarded HTTP calls to the CELLxGENE API (two preset targets on api.cellxgene.cziscience.com) and a short list of local commands: python, Rscript and uv. To inspect an .h5ad file, run Scanpy QC or run a Seurat script, the agent writes the code and runs it through run_singlecellomicsagent_command once you approve the exact command. It has no dedicated AnnData, QC or marker tools.',
    asks: [
      'Write and run a Python script that prints the cell count and obs columns of my data.h5ad.',
      'Plan a Scanpy QC and clustering workflow for this dataset before running anything.',
      'Find CELLxGENE datasets from human lung tissue.'
    ],
    tools: ['get_singlecellomicsagent_status', 'list_singlecellomicsagent_api_targets', 'call_singlecellomicsagent_api', 'plan_singlecellomicsagent_command', 'run_singlecellomicsagent_command', 'summarize_singlecellomicsagent_resource'],
    needs: 'No API keys. Local analysis needs Python or R already installed with the libraries you use (Scanpy, AnnData, Seurat). The extension does not install them. Python 3.11 or later, from the .brxt bundle.',
    sources: [
      'CELLxGENE API (api.cellxgene.cziscience.com)',
      'CELLxGENE Discover (api.cellxgene.cziscience.com/discover/v1)',
      'Local files, through approved python, Rscript or uv runs'
    ]
  },
  opentronsagent: {
    domain: 'Lab & ELN',
    what: 'Sends HTTP requests to an Opentrons robot server (http://localhost:31950 by default), so the agent can read health, run and protocol records. POST, PUT, PATCH and DELETE, which would create or change runs, are blocked unless allow_mutation=true. The only local command it may run is opentrons_simulate, which simulates a protocol file after you approve with allow_execute=true.',
    asks: [
      'Check the health endpoint of my Opentrons robot and report its API version.',
      'Simulate protocol.py with opentrons_simulate and summarize the steps.',
      'List the runs recorded on the robot without starting or changing anything.'
    ],
    tools: ['get_opentronsagent_status', 'list_opentronsagent_api_targets', 'call_opentronsagent_api', 'plan_opentronsagent_command', 'run_opentronsagent_command', 'summarize_opentronsagent_resource'],
    skills: ['opentrons-lab-automation'],
    needs: 'API calls need an Opentrons robot you can reach on your network. The optional OPENTRONS_API_TOKEN (secret) is sent as a Bearer header. Simulation needs the opentrons Python package, so that opentrons_simulate is on PATH.',
    sources: ['Opentrons robot HTTP API (http://localhost:31950 by default)', 'opentrons_simulate on your machine']
  },
  ginkgocloudlabagent: {
    domain: 'Lab & ELN',
    what: 'Makes HTTP requests to https://api.ginkgobioworks.com (two preset targets, ginkgo and ginkgo_custom, with the same base URL) to read project, run, sample and protocol metadata. If GINKGO_API_TOKEN is set, it is sent as a Bearer token. Write methods are blocked unless allow_mutation=true, and it runs no local commands because its command list is empty. The extension defines no Ginkgo endpoints of its own, so the agent supplies the API paths.',
    asks: [
      'Check whether my Ginkgo API token is configured, without printing it.',
      'Fetch the metadata for this cloud lab run ID and list its provenance fields.',
      'Summarize this exported Ginkgo run JSON.'
    ],
    tools: ['get_ginkgocloudlabagent_status', 'list_ginkgocloudlabagent_api_targets', 'call_ginkgocloudlabagent_api', 'plan_ginkgocloudlabagent_command', 'run_ginkgocloudlabagent_command', 'summarize_ginkgocloudlabagent_resource'],
    needs: 'Endpoints that need a login need a Ginkgo cloud lab account and an API token in GINKGO_API_TOKEN (stored as a secret). Nothing needs to be installed locally.',
    sources: ['Ginkgo API (api.ginkgobioworks.com)']
  },
  lamindbagent: {
    domain: 'Lab & ELN',
    what: 'Sends HTTP requests to the LaminDB API (https://lamin.ai/api by default) to read instance, dataset, artifact and collection metadata. If LAMINDB_API_KEY is set, it is sent as a Bearer key. The one local command it may run is the lamin CLI, only after you approve with allow_execute=true, and write methods are blocked unless allow_mutation=true.',
    asks: [
      'Run lamin to show which LaminDB instance I am connected to.',
      'List the artifacts in my LaminDB instance with their versions.',
      'Summarize the lineage and version fields in this exported collection record.'
    ],
    tools: ['get_lamindbagent_status', 'list_lamindbagent_api_targets', 'call_lamindbagent_api', 'plan_lamindbagent_command', 'run_lamindbagent_command', 'summarize_lamindbagent_resource'],
    needs: 'A LaminDB account and instance. LAMINDB_API_KEY (secret) and LAMINDB_INSTANCE are optional. The lamin command needs lamindb installed, so that lamin is on PATH.',
    sources: ['LaminDB API (lamin.ai/api)', 'The lamin CLI on your machine']
  },
  cellxgenecensusagent: {
    domain: 'Genomics & omics',
    what: 'Makes HTTP requests to api.cellxgene.cziscience.com (two preset targets, cellxgene_census and cellxgene_discover, with the same base URL) to find datasets, collections and cell metadata. Write methods are blocked unless allow_mutation=true. It sends no credentials and runs no local commands. It does not use the cellxgene_census Python package, so Census expression data is outside its reach.',
    asks: [
      'List public CELLxGENE collections that include human brain data.',
      'Get the metadata for this CELLxGENE dataset ID and list its assays and tissues.',
      'Summarize this exported collection JSON and keep the dataset IDs.'
    ],
    tools: ['get_cellxgenecensusagent_status', 'list_cellxgenecensusagent_api_targets', 'call_cellxgenecensusagent_api', 'plan_cellxgenecensusagent_command', 'run_cellxgenecensusagent_command', 'summarize_cellxgenecensusagent_resource'],
    needs: 'Nothing. No account or API key. CELLXGENE_CENSUS_VERSION is an optional label.',
    sources: ['CELLxGENE API (api.cellxgene.cziscience.com)']
  },
  depmapagent: {
    domain: 'Genomics & omics',
    what: 'Sends HTTP requests to two DepMap targets, the portal API (https://depmap.org/portal/api) and the download area (https://depmap.org/portal/download), and returns the status code with the JSON or text body. GET, HEAD and OPTIONS run freely, and POST, PUT, PATCH and DELETE are refused unless the call passes allow_mutation=true. A summarize tool lists the top level keys and the ID, version and URL fields of a JSON payload you already have, with no network call.',
    asks: [
      'Check the status of DepMapAgent and list the API targets it can reach.',
      'Ask the DepMap portal API which datasets are available, and keep the source URL in the answer.',
      'Summarize this exported DepMap JSON and tell me which provenance fields it carries.'
    ],
    tools: ['get_depmapagent_status', 'list_depmapagent_api_targets', 'call_depmapagent_api', 'plan_depmapagent_command', 'run_depmapagent_command', 'summarize_depmapagent_resource'],
    needs: 'Nothing required. Optional DEPMAP_API_KEY (secret, sent as an Authorization: Bearer header). DEPMAP_LOG_LEVEL is INFO by default. Python 3.11 or later, from the .brxt bundle, and network access to depmap.org.',
    sources: ['DepMap portal API and downloads (depmap.org)']
  },
  primekgagent: {
    domain: 'Knowledge graphs',
    what: 'Has two HTTP targets: raw files from the mims-harvard/PrimeKG GitHub repository, and the GitHub REST API. It returns the response body for read requests and blocks write methods unless allow_mutation=true. It does not load the graph or run graph queries. A summarize tool reports the top level keys and ID fields of a JSON payload you pass in.',
    asks: [
      'Fetch the PrimeKG README from its GitHub repository and summarize what the graph contains.',
      'Use the GitHub API target to list the files in the mims-harvard/PrimeKG repository.',
      'Check the status of PrimeKGAgent and show which targets and environment variables are configured.'
    ],
    tools: ['get_primekgagent_status', 'list_primekgagent_api_targets', 'call_primekgagent_api', 'plan_primekgagent_command', 'run_primekgagent_command', 'summarize_primekgagent_resource'],
    needs: 'Nothing. It sends no credentials. PRIMEKG_LOG_LEVEL is INFO by default. Python 3.11 or later, from the .brxt bundle, and network access to GitHub.',
    sources: ['PrimeKG repository files (raw.githubusercontent.com/mims-harvard/PrimeKG)', 'GitHub REST API (api.github.com)']
  },
  zoteroagent: {
    domain: 'Literature',
    what: 'Has one HTTP target, the Zotero Web API (https://api.zotero.org). The model builds the request path, for example a user or group library path, and gets back the JSON body. POST, PUT, PATCH and DELETE are refused unless allow_mutation=true. A summarize tool reports the top level keys and ID fields of a Zotero JSON export you already have.',
    asks: [
      'List the collections in my Zotero user library (my user ID is 1234567).',
      'Find items tagged multiple sclerosis in our group library and return their titles and DOIs.',
      'Summarize this exported Zotero item JSON and show its identifiers.'
    ],
    tools: ['get_zoteroagent_status', 'list_zoteroagent_api_targets', 'call_zoteroagent_api', 'plan_zoteroagent_command', 'run_zoteroagent_command', 'summarize_zoteroagent_resource'],
    needs: 'Optional ZOTERO_API_KEY (secret, sent as Authorization: Bearer). In practice a private library needs a key. ZOTERO_USER_ID and ZOTERO_GROUP_ID are declared, but the code only reports whether they are set, so the library ID must appear in the request path. ZOTERO_LOG_LEVEL is INFO by default. Python 3.11 or later, from the .brxt bundle.',
    sources: ['Zotero Web API (api.zotero.org)']
  },
  markitdownagent: {
    domain: 'Productivity',
    what: 'Runs one local command, markitdown, to convert documents to Markdown. The run tool executes it without a shell, with the arguments, working directory and time limit you approve, and only when allow_execute=true. It returns the exit code and the last 20,000 characters of stdout and stderr. It has no API targets, and the status tool reports whether markitdown is on PATH.',
    asks: [
      'Check whether the markitdown command is installed.',
      'Convert report.pdf in ~/Documents to Markdown with markitdown.',
      'Plan the markitdown command to convert slides.pptx and show me what will run before running it.'
    ],
    tools: ['get_markitdownagent_status', 'list_markitdownagent_api_targets', 'call_markitdownagent_api', 'plan_markitdownagent_command', 'run_markitdownagent_command', 'summarize_markitdownagent_resource'],
    needs: 'The markitdown command line tool, installed separately and on PATH. This package does not install it. No credentials. Each conversion needs allow_execute=true. MARKITDOWN_LOG_LEVEL is INFO by default. Python 3.11 or later, from the .brxt bundle.',
    sources: ['Local documents you name']
  },
  adaptyvbiofoundryagent: {
    domain: 'Lab & ELN',
    what: 'Has one HTTP target, https://api.adaptyvbio.com, which ADAPTYV_API_BASE_URL can override. Read requests run freely. Submissions and any POST, PUT, PATCH or DELETE request are refused unless the call passes allow_mutation=true after you approve. A summarize tool reports the top level keys and the ID, status and timestamp fields of a JSON payload, and its skill tells the model to keep wet lab actions as plans until you approve them.',
    asks: [
      'Check the status of AdaptyvBioFoundryAgent and confirm whether an API key is configured.',
      'Fetch the status and result metadata for my Adaptyv experiment and keep the job IDs.',
      'Draft the request to submit this sequence library to Adaptyv, but do not send it.'
    ],
    tools: ['get_adaptyvbiofoundryagent_status', 'list_adaptyvbiofoundryagent_api_targets', 'call_adaptyvbiofoundryagent_api', 'plan_adaptyvbiofoundryagent_command', 'run_adaptyvbiofoundryagent_command', 'summarize_adaptyvbiofoundryagent_resource'],
    needs: 'Optional ADAPTYV_API_KEY (secret, sent as Authorization: Bearer). Endpoints that need a login need an Adaptyv account and key. Optional ADAPTYV_API_BASE_URL. ADAPTYV_LOG_LEVEL is INFO by default. Python 3.11 or later, from the .brxt bundle.',
    sources: ['Adaptyv Bio API (api.adaptyvbio.com)']
  },
  structuredpapersearchagent: {
    domain: 'Literature',
    what: 'Has five scholarly API targets: OpenAlex, Semantic Scholar Graph v1, Crossref, NCBI E-utilities (PubMed) and bioRxiv. The model picks a target and a path and gets back the JSON or text response, and its skill tells it to keep DOIs, PMIDs and URLs in the answer. Write methods are refused unless allow_mutation=true, and a summarize tool lists ID fields such as doi, pmid and pmcid in a payload.',
    asks: [
      'Search OpenAlex for 2024 papers on remyelination therapy and list their DOIs.',
      'Look up PMID 12345678 through PubMed E-utilities and return the title, journal and year.',
      'Get Semantic Scholar metadata for this DOI and show its citation count.'
    ],
    tools: ['get_structuredpapersearchagent_status', 'list_structuredpapersearchagent_api_targets', 'call_structuredpapersearchagent_api', 'plan_structuredpapersearchagent_command', 'run_structuredpapersearchagent_command', 'summarize_structuredpapersearchagent_resource'],
    needs: 'Nothing required. Optional SEMANTIC_SCHOLAR_API_KEY (secret, sent as x-api-key to Semantic Scholar) and PAPER_SEARCH_CONTACT_EMAIL (sent as the From header that some APIs ask for). STRUCTURED_PAPER_SEARCH_LOG_LEVEL is INFO by default. Python 3.11 or later, from the .brxt bundle.',
    sources: ['OpenAlex', 'Semantic Scholar Graph API v1', 'Crossref', 'PubMed, through NCBI E-utilities', 'bioRxiv']
  },
  scientificwebresearchagent: {
    domain: 'Web & browser',
    what: 'Has two HTTP targets, the Exa API (https://api.exa.ai) and the Tavily API (https://api.tavily.com), plus a custom mode that fetches any full URL. It returns the response body and URL so the model can cite sources, and its skill tells it to keep what the API reported apart from its own inference. POST, PUT, PATCH and DELETE are refused unless allow_mutation=true.',
    asks: [
      'Search the web with Tavily for recent guidance on CAR-T neurotoxicity and list the source URLs.',
      'Use Exa to find lab protocols for organoid culture outside journal databases.',
      'Fetch this conference page and summarize it, keeping the URL for citation.'
    ],
    tools: ['get_scientificwebresearchagent_status', 'list_scientificwebresearchagent_api_targets', 'call_scientificwebresearchagent_api', 'plan_scientificwebresearchagent_command', 'run_scientificwebresearchagent_command', 'summarize_scientificwebresearchagent_resource'],
    needs: 'EXA_API_KEY, TAVILY_API_KEY or both (secret). The manifest marks both optional, but a search needs a key for the service it uses. SCIENTIFIC_WEB_RESEARCH_LOG_LEVEL is INFO by default. Python 3.11 or later, from the .brxt bundle.',
    sources: ['Exa API (api.exa.ai)', 'Tavily API (api.tavily.com)', 'Any URL you give it']
  },
  tamarindbioagent: {
    domain: 'Chemistry & structure',
    what: 'Sends HTTP requests to the Tamarind Bio API (https://api.tamarind.bio by default). It has no dedicated folding, design or docking tools: the model picks an API path and calls it through one general tool. GET, HEAD and OPTIONS run freely, but POST, PUT, PATCH and DELETE, such as submitting a job, are refused unless you approve and allow_mutation=true is passed. A local tool then lists the keys of a returned JSON result and any ID, status, URL, version or timestamp fields, so they can be cited.',
    asks: [
      'Check whether TamarindBioAgent has an API key set and which base URL it will call.',
      'List my Tamarind jobs with the name and status of each.',
      'Draft the request to submit this docking job to Tamarind, and wait for my approval before sending it.'
    ],
    tools: ['get_tamarindbioagent_status', 'list_tamarindbioagent_api_targets', 'call_tamarindbioagent_api', 'summarize_tamarindbioagent_resource', 'plan_tamarindbioagent_command', 'run_tamarindbioagent_command'],
    needs: 'uv, because Biorouter runs uv sync when it installs a .brxt (Python 3.11 or later). TAMARIND_API_KEY is optional and secret. When it is set it is sent as an Authorization: Bearer header, and without it requests are anonymous. The key comes from your own Tamarind Bio account. TAMARIND_API_BASE_URL is optional and points the agent at another compatible API. TAMARIND_LOG_LEVEL is INFO by default. Network access.',
    sources: ['Tamarind Bio API (api.tamarind.bio)']
  },
  rowanmolecularmodelingagent: {
    domain: 'Chemistry & structure',
    what: 'Sends HTTP requests to the Rowan API (https://api.rowansci.com by default). Calculations such as conformers, pKa, docking or cofolding are whatever the Rowan API offers, reached through one general call tool, and the extension has no tool for each calculation. Read methods run freely, but POST, PUT, PATCH and DELETE, such as starting a calculation, are refused unless you approve and allow_mutation=true is passed. A local tool lists the keys of a returned JSON result and any ID, status, URL or version fields, so they can be cited.',
    asks: [
      'Show me the status of my recent Rowan workflows, with their IDs.',
      'Fetch the results of a Rowan workflow by its ID and summarize which fields came back.',
      'Prepare a pKa calculation request for this SMILES, but do not submit it until I approve.'
    ],
    tools: ['get_rowanmolecularmodelingagent_status', 'list_rowanmolecularmodelingagent_api_targets', 'call_rowanmolecularmodelingagent_api', 'summarize_rowanmolecularmodelingagent_resource', 'plan_rowanmolecularmodelingagent_command', 'run_rowanmolecularmodelingagent_command'],
    needs: 'uv, because Biorouter runs uv sync when it installs a .brxt (Python 3.11 or later). ROWAN_API_KEY is optional and secret. When it is set it is sent as an Authorization: Bearer header, and without it requests are anonymous. The key comes from your own Rowan account. ROWAN_API_BASE_URL is optional and points the agent at another compatible API. ROWAN_LOG_LEVEL is INFO by default. Network access.',
    sources: ['Rowan API (api.rowansci.com)']
  },
  unifiedpublicdatabaselookupagent: {
    domain: 'Genomics & omics',
    what: 'Sends HTTP requests to eight public APIs: NCBI E-utilities, UniProt REST, Ensembl REST, OpenAlex, Crossref, ChEMBL, PubChem PUG REST and SEC data.sec.gov. It can also call any other full URL through target=\'custom\'. It is for looking up identifiers and metadata. GET, HEAD and OPTIONS run freely, while write methods need your approval and allow_mutation=true. The model chooses the API path, and a local tool lists the keys of a returned JSON record and any accession, DOI, PMID, ID or URL fields, so the answer can cite where each fact came from.',
    asks: [
      'Look up UniProt entry P04637 and give me the accession and source URL.',
      'Find the PubChem CID and the ChEMBL ID for imatinib.',
      'Get the Crossref metadata for DOI 10.1038/nature12373.'
    ],
    tools: ['get_unifiedpublicdatabaselookupagent_status', 'list_unifiedpublicdatabaselookupagent_api_targets', 'call_unifiedpublicdatabaselookupagent_api', 'summarize_unifiedpublicdatabaselookupagent_resource', 'plan_unifiedpublicdatabaselookupagent_command', 'run_unifiedpublicdatabaselookupagent_command'],
    needs: 'No API keys or accounts. uv, because Biorouter runs uv sync when it installs a .brxt (Python 3.11 or later), and network access. PUBLIC_DB_CONTACT_EMAIL is optional. When it is set it is sent as an HTTP From header, for APIs that ask callers to identify themselves. UNIFIED_PUBLIC_DB_LOG_LEVEL is INFO by default.',
    sources: [
      'NCBI E-utilities', 'UniProt REST', 'Ensembl REST', 'OpenAlex', 'Crossref', 'ChEMBL', 'PubChem PUG REST', 'SEC data.sec.gov'
    ]
  }
};
