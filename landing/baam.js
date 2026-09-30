/* BAAM marketplace page behaviour.

   The static rows in baam.html are both the no-JS view and the input of
   landing/scripts/build-registry.mjs, which generates registry.json and the
   app's compiled-in privacy set from them. This script renders the extension
   shelf from registry.json over those rows (all or nothing), filters the three
   shelves, and opens a row in place.

   Pinned by landing/scripts/baam-privacy-facet.test.mjs and
   baam-search.test.mjs. Filtering is synchronous on the input event, cards are
   hidden with inline display:none, and the privacy facet matches the card's own
   data-privacy, never its prose. */

  function escapeHtml(value) {
    return String(value == null ? '' : value).replace(/[&<>"']/g, ch => ({
      '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
    }[ch]));
  }

  /* ─────────────────────────────────────────────────────────────
     Tabs / shelves
     ───────────────────────────────────────────────────────────── */
  const SHELVES = ['extensions', 'skills', 'workflows'];
  let activeShelf = 'extensions';
  const PLACEHOLDER = {
    extensions: 'Search extensions',
    skills: 'Search skills',
    workflows: 'Search workflows'
  };

  function setShelf(name, updateHash) {
    if (!SHELVES.includes(name)) name = 'extensions';
    activeShelf = name;
    document.querySelectorAll('.shelf').forEach(s => { s.hidden = (s.dataset.shelf !== name); });
    document.querySelectorAll('.baam-tab').forEach(t => {
      const on = t.dataset.shelf === name;
      t.classList.toggle('active', on);
      t.setAttribute('aria-selected', on ? 'true' : 'false');
      t.tabIndex = on ? 0 : -1;
    });
    const si = document.getElementById('baam-search');
    if (si) si.placeholder = PLACEHOLDER[name];
    if (updateHash) { try { history.replaceState(null, '', '#' + name); } catch (e) {} }
    runFilter();
  }

  function shelfFromHash() {
    const h = (location.hash || '').replace('#', '').toLowerCase();
    if (h.indexOf('extension') === 0) return 'extensions';
    if (h.indexOf('skill') === 0) return 'skills';
    if (h.indexOf('workflow') === 0) return 'workflows';
    return null;
  }

  function runFilter() {
    const q = (document.getElementById('baam-search').value || '').toLowerCase().trim();
    if (activeShelf === 'extensions') filterExtensions(q);
    else if (activeShelf === 'skills') filterSkills(q);
    else filterWorkflows(q);
    // Rows a filter just revealed are measured now that they have a width.
    clampAll();
  }

  /* ── Text search (shared by all three shelves) ─────────────────
     The matcher is landing/marketplace-search.js, a port of the app's own
     crates/biorouter/src/catalog_search.rs. Facet chips still match against
     `searchHaystack`: they are exact values or curated substrings a chip
     authored, not prose a person typed. */

  /* The searched text of one card, weighted. Deliberately NOT `textContent`:
     that sweeps in the download button, the licence chip, the org line and an
     open row's detail panel, and none of those is a searched field in the
     canonical matcher. */
  function cardFields(card) {
    var W = window.MarketplaceSearch.Weight;
    var fields = [];
    var push = function (el, weight) { if (el) fields.push([el.textContent, weight]); };
    // The catalog's own ID and manifest name, at Name weight: the two fields
    // `catalog_search.rs` searches and this page renders nowhere. Read from the
    // attribute rather than derived from the download filename, because
    // `spokeagent-0.4.1.brxt` is exactly why `data-registry-id` exists.
    if (card.dataset.registryId) fields.push([card.dataset.registryId, W.Name]);
    if (card.dataset.extensionName) fields.push([card.dataset.extensionName, W.Name]);
    push(card.querySelector('h3'), W.Name);
    // ⚠ The `.skill-type` line, but only its SLUG. The line is
    // "User-invocable · /scientific-research", or "5 skills · auto-applied".
    // The slug is the skill's registry id, which the app searches at Name
    // weight; everything else in it is the invocation MODE, which is a facet
    // (measured: searching the mode made `invocable` show 62 of 132 skills and
    // `auto` 74, against 0 and 4 in the app).
    var typeLine = card.querySelector('.skill-type');
    if (typeLine) {
      String(typeLine.textContent).split('·').forEach(function (part) {
        var text = part.trim();
        if (text.charAt(0) === '/') fields.push([text, W.Name]);
      });
    }
    push(card.querySelector('.ext-desc, .skill-desc, .wf-desc'), W.Prose);
    // The organization, WITHOUT the version. "BaranziniLab · UCSF · v0.2.0"
    // splits into `v0`, `2`, `0`, so a bare `2` or `0` (which is what
    // "Apache-2.0" tokenises to) matched almost every card through its version.
    var org = card.querySelector('.ext-org');
    var version = '';
    if (org) {
      var orgText = String(org.textContent);
      var tail = /[·|]\s*(v[\d.]+)\s*$/.exec(orgText);
      if (tail) version = tail[1];
      fields.push([orgText.replace(/[·|]\s*v[\d.]+\s*$/, ''), W.Label]);
    }
    // ⚠ Not `data-tags` as it stands. That blob is
    // `name + organization + version + description + tags`, so it carries the
    // licence and the version, and both were measured matching almost
    // everything. Both are dropped by the card's OWN value, never by a pattern:
    // blanking "anything shaped like a version" reads a bare number such as
    // `13485` (ISO 13485, a skill keyword) as a version.
    var licenceWords = wordsOf(card.dataset.license);
    var versionWords = wordsOf(version);
    if (card.dataset.tags) {
      fields.push([
        String(card.dataset.tags).replace(/[^\s,]+/g, function (t) {
          if (saysOnly(t, licenceWords)) return '';
          return saysOnly(t, versionWords) ? '' : t;
        }),
        W.Label,
      ]);
    }
    // The visible chips, minus the two badges that answer facets rather than
    // words, and minus the licence chip, identified by the card's own
    // data-license so a new licence needs no edit.
    card.querySelectorAll('.ext-tags .tag, .skill-tags .tag, .wf-tags .tag').forEach(function (chip) {
      if (chip.hasAttribute('data-privacy-badge') || chip.hasAttribute('data-affiliation-badge')) return;
      if (saysOnly(chip.textContent, licenceWords)) return;
      fields.push([chip.textContent, W.Label]);
    });
    return fields;
  }

  /* The cards `q` matches, as a Set. An empty query matches everything. The
     shelves are NOT reordered by rank: DOM order is editorial. */
  function searchMatches(q, cards, noise) {
    return window.MarketplaceSearch.matching(q, noise, cards, cardFields);
  }

  /* ── Chip helpers (shared by both chip bars) ───────────────── */
  function onChipClick(bar, btn) {
    if (!btn.dataset.facet) {
      bar.querySelectorAll('.fchip').forEach(c => { c.classList.remove('active'); c.setAttribute('aria-pressed', 'false'); });
      btn.classList.add('active');
      btn.setAttribute('aria-pressed', 'true');
    } else {
      btn.classList.toggle('active');
      btn.setAttribute('aria-pressed', btn.classList.contains('active') ? 'true' : 'false');
      const allBtn = bar.querySelector('.fchip[data-facet=""]');
      const anyActive = bar.querySelector('.fchip.active:not([data-facet=""])');
      if (allBtn) { allBtn.classList.toggle('active', !anyActive); allBtn.setAttribute('aria-pressed', anyActive ? 'false' : 'true'); }
    }
    runFilter();
  }
  function activeChips(bar) {
    const map = {};
    if (!bar) return map;
    bar.querySelectorAll('.fchip.active').forEach(c => {
      if (!c.dataset.facet) return;
      (map[c.dataset.facet] = map[c.dataset.facet] || []).push(c.dataset.match);
    });
    return map;
  }

  /* ─────────────────────────────────────────────────────────────
     Extensions: rendered client-side from registry.json
     ───────────────────────────────────────────────────────────── */
  const FEATURED_EXT_IDS = ['cdwagent', 'ucsfomopagent', 'spokeagent', 'codegraphagent'];
  // Registry ids can carry a version suffix (e.g. "spokeagent-0.4.1"), so match on the base slug.
  const featIndex = id => {
    id = (id || '').toLowerCase();
    for (let i = 0; i < FEATURED_EXT_IDS.length; i++) {
      const b = FEATURED_EXT_IDS[i];
      if (id === b || id.indexOf(b + '-') === 0) return i;
    }
    return -1;
  };
  const isAdapterExt = ext => /BRXT/i.test(ext.organization || '');
  let adaptersManualOpen = false;

  // An entry may omit `privacy` (an un-annotated card is public by
  // construction, and the generator resolves it that way), but an entry that
  // STATES a tier must state one of the two that exist. Absent and unreadable
  // are different things: calling an unreadable tier "public" is a reassurance
  // nobody computed. See readableTiers().
  const tierOf = (extension) => {
    if (!('privacy' in extension) || extension.privacy === undefined || extension.privacy === null) {
      return 'public';
    }
    return extension.privacy === 'private' || extension.privacy === 'public' ? extension.privacy : null;
  };

  // registry.json is fetched at runtime from a server this page cannot re-run
  // the generator against, so the generator's refusals do not protect the
  // renderer. One unreadable tier discards the whole payload, and the fallback
  // is the authored static rows, which carry their own badges.
  function readableTiers(extensions) {
    const bad = extensions.filter(e => tierOf(e) === null);
    if (bad.length === 0) return true;
    console.warn(
      'baam: keeping the static extension shelf. ' + bad.length +
      ' registry entr' + (bad.length === 1 ? 'y states a tier that is neither' : 'ies state a tier that is neither') +
      ' "private" nor "public": ' + bad.map(e => e.id + '=' + JSON.stringify(e.privacy)).join(', ')
    );
    return false;
  }

  // The affiliation a registry entry DECLARES, as a list. Absent means
  // unconstrained; readableAffiliations() has already refused anything present
  // and unusable.
  const affiliationOf = (extension) =>
    Array.isArray(extension.affiliation) ? extension.affiliation : [];

  // The badge label: the display name out of the registry's `institutions`
  // map, never the raw id. The " data" keeps it from reading as the ordinary
  // "UCSF" org tag.
  const institutionBadgeLabel = (institutions, id) => institutions[id] + ' data';

  // The affiliation half of readableTiers(). The generator hard-fails on an
  // empty affiliation, on one naming an institution the map does not declare,
  // and on one on a public card; rendering must not paper over any of them.
  function readableAffiliations(extensions, institutions) {
    const names = institutions && typeof institutions === 'object' ? institutions : {};
    const bad = [];
    extensions.forEach(e => {
      const declared = e.affiliation;
      if (declared === undefined || declared === null || !('affiliation' in e)) return;
      if (!Array.isArray(declared) || declared.length === 0) {
        bad.push(e.id + ': affiliation is ' + JSON.stringify(declared) +
          '; absent means unconstrained, so an empty one would render a constraint nobody declared');
        return;
      }
      if (tierOf(e) !== 'private') {
        bad.push(e.id + ': affiliation on a public extension; affiliation asks under whose ' +
          'agreements, which only arises once the data is private');
        return;
      }
      declared.forEach(id => {
        if (typeof names[id] !== 'string' || names[id].length === 0) {
          bad.push(e.id + ': affiliation names ' + JSON.stringify(id) +
            ', which the registry\'s institutions map does not name');
        }
      });
    });
    if (bad.length === 0) return true;
    console.warn(
      'baam: keeping the static extension shelf. ' + bad.length +
      ' registry affiliation' + (bad.length === 1 ? '' : 's') + ' cannot be rendered: ' + bad.join('; ')
    );
    return false;
  }

  /* ── Row glyphs: a small line icon for each kind of extension ── */
  const GLYPHS = {
    'Clinical data': '<ellipse cx="8" cy="3.75" rx="5" ry="1.9"/><path d="M3 3.75v8.5c0 1.05 2.24 1.9 5 1.9s5-.85 5-1.9v-8.5"/><path d="M3 8c0 1.05 2.24 1.9 5 1.9S13 9.05 13 8"/>',
    'Knowledge graphs': '<circle cx="3.75" cy="4" r="1.75"/><circle cx="12.25" cy="4" r="1.75"/><circle cx="8" cy="12.25" r="1.75"/><path d="M5.5 4h5M4.6 5.6l2.5 5M11.4 5.6l-2.5 5"/>',
    'Code': '<path d="M5.5 4.25 1.75 8l3.75 3.75M10.5 4.25 14.25 8l-3.75 3.75"/>',
    'Productivity': '<path d="M9.25 1.75H4.5A1.25 1.25 0 0 0 3.25 3v10a1.25 1.25 0 0 0 1.25 1.25h7A1.25 1.25 0 0 0 12.75 13V5.25z"/><path d="M9.25 1.75v3.5h3.5M5.75 8.5h4.5M5.75 11h4.5"/>',
    'Lab & ELN': '<path d="M6 1.75h4M6.75 1.75v4.4L3 12.55a1.1 1.1 0 0 0 .95 1.7h8.1a1.1 1.1 0 0 0 .95-1.7L9.25 6.15v-4.4"/><path d="M4.4 10.25h7.2"/>',
    'Compute & workflows': '<rect x="2" y="2.25" width="12" height="4.75" rx="1.25"/><rect x="2" y="9" width="12" height="4.75" rx="1.25"/><path d="M4.75 4.6h.01M4.75 11.4h.01"/>',
    'Imaging': '<rect x="2" y="2.5" width="12" height="11" rx="1.5"/><circle cx="6" cy="6.25" r="1.25"/><path d="m14 10.75-3.25-3.25L4 13.5"/>',
    'Literature': '<path d="M8 4C6.5 2.9 4.5 2.5 2 2.75v9.5c2.5-.25 4.5.15 6 1.25 1.5-1.1 3.5-1.5 6-1.25v-9.5C11.5 2.5 9.5 2.9 8 4zM8 4v9.5"/>',
    'Genomics & omics': '<path d="M4.5 1.75c0 4.2 7 4.3 7 12.5M11.5 1.75c0 4.2-7 4.3-7 12.5"/><path d="M5.6 4.25h4.8M5.6 11.75h4.8M6.9 8h2.2"/>',
    'Chemistry & structure': '<path d="M8 1.75 13.4 4.9v6.2L8 14.25 2.6 11.1V4.9z"/><circle cx="8" cy="8" r="2"/>',
    'Web & browser': '<circle cx="8" cy="8" r="6.25"/><path d="M1.75 8h12.5M8 1.75c1.75 1.75 2.6 3.8 2.6 6.25S9.75 12.5 8 14.25C6.25 12.5 5.4 10.45 5.4 8S6.25 3.5 8 1.75z"/>'
  };
  /* Biomedical skill rows get a glyph for their kind of analysis, so the 63
     rows of that group do not repeat one picture. Keyed by the download slug;
     a slug missing here keeps the row's own icon. */
  const BIO_GLYPHS = {
    reads: '<path d="M2 4.5h7.5M6.5 8H14M2 11.5h9"/>',
    helix: GLYPHS['Genomics & omics'],
    wave: '<path d="M2 8c2-3.5 4-3.5 6 0s4 3.5 6 0"/><path d="M2 12.5h12" opacity=".45"/>',
    cells: '<circle cx="5.25" cy="5.5" r="2.5"/><circle cx="11.25" cy="4.75" r="1.75"/><circle cx="9.5" cy="11" r="2.75"/>',
    molecule: GLYPHS['Chemistry & structure'],
    network: GLYPHS['Knowledge graphs'],
    chart: '<path d="M2 13.5h12M4.5 13.5V9.5M8 13.5V3.5M11.5 13.5V7"/>',
    database: GLYPHS['Clinical data'],
    tree: '<path d="M1.75 7.5H5M5 3.5v8M5 3.5h9M5 11.5h3M8 8.75v5.5M8 8.75h6M8 14.25h6"/>',
    microbe: '<circle cx="8" cy="8" r="6.25"/><circle cx="6" cy="6.25" r="1"/><circle cx="10.25" cy="7.25" r="1.25"/><circle cx="7.25" cy="10.5" r=".75"/>',
    matrix: '<rect x="2.25" y="2.25" width="11.5" height="11.5" rx="1.5"/><path d="M2.25 6.1h11.5M2.25 9.9h11.5M6.1 2.25v11.5M9.9 2.25v11.5"/>',
    imaging: GLYPHS['Imaging'],
    pipeline: GLYPHS['Compute & workflows'],
    doc: GLYPHS['Productivity'],
    flask: GLYPHS['Lab & ELN']
  };
  const BIO_KIND = {
    'alignment': 'reads', 'alignment-files': 'reads', 'alternative-splicing': 'wave', 'atac-seq': 'helix',
    'causal-genomics': 'helix', 'chemoinformatics': 'molecule', 'chip-seq': 'helix', 'clinical-biostatistics': 'chart',
    'clinical-databases': 'database', 'clip-seq': 'wave', 'comparative-genomics': 'tree', 'copy-number': 'helix',
    'crispr-screens': 'flask', 'data-visualization': 'chart', 'database-access': 'database', 'differential-expression': 'wave',
    'ecological-genomics': 'tree', 'epidemiological-genomics': 'tree', 'epitranscriptomics': 'wave', 'experimental-design': 'chart',
    'expression-matrix': 'matrix', 'flow-cytometry': 'cells', 'gene-regulatory-networks': 'network', 'genome-annotation': 'helix',
    'genome-assembly': 'reads', 'genome-engineering': 'helix', 'genome-intervals': 'reads', 'hi-c-analysis': 'matrix',
    'imaging-mass-cytometry': 'imaging', 'immunoinformatics': 'cells', 'liquid-biopsy': 'helix', 'long-read-sequencing': 'reads',
    'machine-learning': 'network', 'metabolomics': 'molecule', 'metagenomics': 'microbe', 'methylation-analysis': 'helix',
    'microbiome': 'microbe', 'multi-omics-integration': 'network', 'pathway-analysis': 'network', 'phasing-imputation': 'reads',
    'phylogenetics': 'tree', 'population-genetics': 'chart', 'primer-design': 'reads', 'proteomics': 'molecule',
    'read-alignment': 'reads', 'read-qc': 'reads', 'reporting': 'doc', 'restriction-analysis': 'reads',
    'ribo-seq': 'wave', 'rna-quantification': 'wave', 'rna-structure': 'wave', 'sequence-io': 'reads',
    'sequence-manipulation': 'reads', 'single-cell': 'cells', 'small-rna-seq': 'wave', 'spatial-transcriptomics': 'imaging',
    'structural-biology': 'molecule', 'systems-biology': 'network', 'tcr-bcr-analysis': 'cells', 'temporal-genomics': 'chart',
    'variant-calling': 'helix', 'workflow-management': 'pipeline', 'workflows': 'pipeline'
  };
  function bioGlyph(card) {
    const dl = card.querySelector('.skill-dl-btn');
    const kind = dl && BIO_KIND[slugOf(dl.getAttribute('href'))];
    const tile = card.querySelector('.icon-tile');
    if (!kind || !tile) return;
    tile.innerHTML = `<svg class="glyph" viewBox="0 0 16 16" aria-hidden="true">${BIO_GLYPHS[kind]}</svg>`;
  }
  const GLYPH_DEFAULT = '<rect x="2.25" y="2.25" width="11.5" height="11.5" rx="2"/><path d="M8 5.25v5.5M5.25 8h5.5"/>';
  function detailsFor(id) { return (window.BAAM_DETAILS || {})[id] || null; }
  function glyphFor(id) {
    const d = detailsFor(id);
    const paths = (d && GLYPHS[d.domain]) || GLYPH_DEFAULT;
    return `<svg class="glyph" viewBox="0 0 16 16" aria-hidden="true">${paths}</svg>`;
  }
  const GH_SVG = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 2C6.48 2 2 6.48 2 12c0 4.42 2.87 8.17 6.84 9.49.5.09.68-.22.68-.48v-1.7c-2.78.6-3.37-1.34-3.37-1.34-.45-1.16-1.11-1.47-1.11-1.47-.91-.62.07-.61.07-.61 1 .07 1.53 1.03 1.53 1.03.9 1.52 2.34 1.08 2.91.83.09-.65.35-1.08.63-1.33-2.22-.25-4.55-1.11-4.55-4.94 0-1.09.39-1.98 1.03-2.68-.1-.25-.45-1.27.1-2.64 0 0 .84-.27 2.75 1.02A9.56 9.56 0 0 1 12 6.8c.85 0 1.71.11 2.51.33 1.91-1.29 2.75-1.02 2.75-1.02.55 1.37.2 2.39.1 2.64.64.7 1.03 1.59 1.03 2.68 0 3.84-2.34 4.68-4.57 4.93.36.31.68.92.68 1.85v2.74c0 .27.18.58.69.48A10.01 10.01 0 0 0 22 12c0-5.52-4.48-10-10-10z"/></svg>';
  const DL_SVG = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 2.5v8M4.5 7 8 10.5 11.5 7M3 13.5h10"/></svg>';
  const CHEV_SVG = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="m4 6 4 4 4-4"/></svg>';

  function toggleButton(name) {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'row-toggle';
    b.setAttribute('aria-expanded', 'false');
    b.setAttribute('aria-label', 'Details for ' + name);
    b.innerHTML = CHEV_SVG;
    return b;
  }

  function extCardHtml(extension, featured, institutions) {
    const privacy = tierOf(extension);
    const privacyLabel = privacy === 'private' ? 'Private' : 'Public';
    // BOTH states are labelled: an unbadged row would read as "not yet
    // reviewed" rather than "public". `data-privacy-badge` marks this chip as
    // the tier's picture rather than a subject tag; the registry generator
    // skips it by that attribute, never by the class.
    const privacyBadge = `<span class="tag ${privacy}" data-privacy-badge>${privacyLabel}</span>`;
    // The institution badge, one chip per declared affiliation, immediately
    // after the tier it qualifies. Position is load-bearing: `trimTagRows`
    // drops chips from the END. A row with no `data-affiliation` renders
    // NOTHING here; absent means unconstrained.
    const affiliation = affiliationOf(extension);
    const affiliationBadges = affiliation.map(id =>
      `<span class="tag affiliation" data-affiliation-badge="${escapeHtml(id)}">` +
      `${escapeHtml(institutionBadgeLabel(institutions, id))}</span>`
    ).join('');
    // The institution badge and a "UCSF" subject tag say the same word; the
    // badge is the one derived from a declaration, so the tag gives way.
    const affiliationWords = {};
    affiliation.forEach(id => {
      affiliationWords[String(id).toLowerCase()] = 1;
      affiliationWords[String(institutions[id]).toLowerCase()] = 1;
    });
    // .ext-tags is exactly one row. The badge takes a slot, so only TWO real
    // tags follow it, and each institution badge takes one of those two.
    const rawTags = (extension.tags || []).filter(
      t => !/^apache/i.test(t) && !affiliationWords[t.toLowerCase()]
    );
    const budget = Math.max(0, 2 - affiliation.length);
    const tags = privacyBadge + affiliationBadges + rawTags.slice(0, budget).map(tag => {
      const low = tag.toLowerCase();
      const cls = low === 'ucsf' ? 'tag ucsf' : low === 'mcp' ? 'tag mcp' : 'tag';
      return `<span class="${cls}">${escapeHtml(tag)}</span>`;
    }).join('');
    const name = escapeHtml(extension.name);
    const org = escapeHtml([extension.organization, extension.version].filter(Boolean).join(' · '));
    const description = escapeHtml(extension.description);
    const filename = escapeHtml(extension.filename || 'download.brxt');
    const github = escapeHtml(extension.github || '#');
    const download = escapeHtml(extension.download || '#');
    const license = escapeHtml(extension.license || 'Apache-2.0');
    const orgStr = extension.organization || '';
    const orgToken = /BRXT/i.test(orgStr) ? 'brxt' : /UCSF/i.test(orgStr) ? 'ucsf' : '';
    const keywords = escapeHtml([extension.name, extension.organization, extension.version, extension.description, ...(extension.tags || [])].join(' '));
    const extName = escapeHtml(extension.extension_name || '');
    // The catalog's own id, which `cardFields` searches at Name weight.
    const registryId = escapeHtml(extension.id || '');
    const featCls = featured ? ' is-feat' : '';
    // ABSENT, not empty, when there is no affiliation: `data-affiliation=""`
    // is the one spelling the generator refuses outright.
    const affAttr = affiliation.length
      ? ` data-affiliation="${escapeHtml(affiliation.join(' '))}"`
      : '';
    return `<div class="ext-card${featCls}" data-registry-id="${registryId}" data-tags="${keywords}" data-license="${license}" data-org="${orgToken}" data-privacy="${privacy}" data-extension-name="${extName}"${affAttr}>
      <div class="ext-card-header">
        <div class="icon-tile" aria-hidden="true">${glyphFor(extension.id)}</div>
        <div class="ext-meta"><h3>${name}</h3><div class="ext-org">${org}</div></div>
        <a href="${github}" target="_blank" rel="noopener" class="ext-gh-link" title="${name} on GitHub" aria-label="${name} on GitHub">${GH_SVG}</a>
      </div>
      <p class="ext-desc">${description}</p>
      <div class="ext-footer">
        <div class="ext-tags">${tags}</div>
        <a href="${download}" class="brxt-chip" title="Download ${filename}" aria-label="Download ${filename}">${DL_SVG}<span>.brxt</span></a>
      </div>
      <button class="row-toggle" type="button" aria-expanded="false" aria-label="Details for ${name}">${CHEV_SVG}</button>
    </div>`;
  }

  function buildExtChips(extensions) {
    const bar = document.getElementById('ext-chips');
    if (!bar) return;
    const N = extensions.length;
    const freq = {};
    extensions.forEach(e => (e.tags || []).forEach(t => {
      const low = t.toLowerCase();
      if (low === 'mcp' || low === 'ucsf' || /^apache/i.test(t)) return;
      freq[t] = (freq[t] || 0) + 1;
    }));
    const tags = Object.keys(freq)
      .filter(t => freq[t] >= 2 && freq[t] <= N * 0.8)
      .sort((a, b) => freq[b] - freq[a] || a.localeCompare(b))
      .slice(0, 8);
    let html = '<button class="fchip active" type="button" aria-pressed="true" data-facet="" data-match="">All</button>';
    html += '<span class="chip-sep" aria-hidden="true"></span>';
    html += '<button class="fchip" type="button" aria-pressed="false" data-facet="privacy" data-match="private">Private</button>';
    html += '<button class="fchip" type="button" aria-pressed="false" data-facet="privacy" data-match="public">Public</button>';
    html += '<span class="chip-sep" aria-hidden="true"></span>';
    html += '<button class="fchip" type="button" aria-pressed="false" data-facet="org" data-match="ucsf">UCSF</button>';
    html += '<button class="fchip" type="button" aria-pressed="false" data-facet="org" data-match="brxt">Integrations</button>';
    if (tags.length) {
      html += '<span class="chip-sep" aria-hidden="true"></span>';
      html += tags.map(t => `<button class="fchip" type="button" aria-pressed="false" data-facet="tag" data-match="${escapeHtml(t.toLowerCase())}">${escapeHtml(t)}</button>`).join('');
    }
    bar.innerHTML = html;
    bar.querySelectorAll('.fchip').forEach(btn => btn.addEventListener('click', () => onChipClick(bar, btn)));
  }

  // `.ext-tags` is ONE row with `overflow: hidden`, so a chip that wraps is
  // deleted from view. Measure after layout and drop the chips that did not
  // fit, from the end, keeping the privacy badge (index 0) always. Re-run
  // whenever the row's width can change.
  function trimTagRows() {
    document.querySelectorAll('#extensions-section .ext-tags').forEach(row => {
      const chips = row.querySelectorAll('span');
      // `hidden` is a UA rule and loses to `.tag { display: inline-flex }`.
      chips.forEach(c => { c.style.display = ''; });
      for (let i = chips.length - 1; i > 0; i--) {
        if (row.scrollHeight <= row.clientHeight) break;
        chips[i].style.display = 'none';
      }
    });
  }

  function renderExtensions(extensions, institutions) {
    const featured = [], primary = [], adapters = [];
    extensions.forEach(e => {
      if (featIndex(e.id) !== -1) featured.push(e);
      else if (isAdapterExt(e)) adapters.push(e);
      else primary.push(e);
    });
    featured.sort((a, b) => featIndex(a.id) - featIndex(b.id));

    document.getElementById('ext-featured').innerHTML = featured.map(e => extCardHtml(e, true, institutions)).join('');
    document.getElementById('ext-featured-wrap').hidden = featured.length === 0;
    document.getElementById('ext-primary').innerHTML = primary.map(e => extCardHtml(e, false, institutions)).join('');
    const primaryWrap = document.getElementById('ext-primary-wrap');
    if (primaryWrap) primaryWrap.hidden = primary.length === 0;

    const adaptWrap = document.getElementById('ext-adapters-wrap');
    document.getElementById('ext-adapters').innerHTML = adapters.map(e => extCardHtml(e, false, institutions)).join('');
    adaptWrap.hidden = adapters.length === 0;
    if (adapters.length === 0) adaptWrap.dataset.empty = '1';
    document.getElementById('count-adapters').textContent = String(adapters.length);

    const staticGrid = document.getElementById('extensions-grid');
    if (staticGrid) staticGrid.innerHTML = '';

    buildExtChips(extensions);
    updateCounts();
    trimTagRows();
  }

  function applyAdapterVisibility(filtering) {
    const wrap = document.getElementById('ext-adapters-wrap');
    const grid = document.getElementById('ext-adapters');
    const btn = document.getElementById('ext-adapters-toggle');
    if (!wrap || !grid || !btn || wrap.dataset.empty === '1') return;
    if (grid.querySelectorAll('.ext-card').length === 0) return;
    if (filtering) {
      grid.hidden = false;
      btn.hidden = true;
    } else {
      grid.hidden = !adaptersManualOpen;
      btn.hidden = false;
      btn.classList.toggle('open', adaptersManualOpen);
      btn.setAttribute('aria-expanded', adaptersManualOpen ? 'true' : 'false');
      btn.querySelector('.lbl').textContent = adaptersManualOpen
        ? 'Show fewer'
        : ('Show all ' + grid.querySelectorAll('.ext-card').length + ' integrations');
    }
  }

  /* ── What a card is searched BY ─────────────────────────────
     The licence is not part of it. Every card in this catalog is Apache-2.0,
     so it separates nothing, and it reached the haystack three ways:
     `data-license`, an `Apache-2.0` chip, and on a skill the `apache` keyword
     in `data-tags`. Measured on 2026-09-12, `apache`, `Apache-2.0` and `pac`
     each returned every card. The same rule lives in
     crates/biorouter/src/catalog_search.rs (`names_only_the_license`) and
     ui/desktop/src/components/baam/search.ts; this is the third copy. By
     WORDS and not by equality, because the chip says `Apache-2.0` while the
     keyword says `apache`. */
  const WORD_BREAK = /[^\p{Alphabetic}\p{N}]+/u;

  function wordsOf(text) {
    return String(text == null ? '' : text).toLowerCase().split(WORD_BREAK).filter(Boolean);
  }

  /* Does `label` say nothing that `words` does not? An empty label says
     nothing at all, which is not the same as saying only these words, so it
     is kept. */
  function saysOnly(label, words) {
    const labelWords = wordsOf(label);
    return labelWords.length > 0 && labelWords.every(w => words.indexOf(w) !== -1);
  }

  function searchHaystack(card) {
    const licenseWords = wordsOf(card.dataset.license);
    // The row's own text, without the detail panel an open row carries: the
    // tag facets answer from what a row says, not from how far a visitor has
    // opened it.
    let text = '';
    card.childNodes.forEach(n => {
      if (n.nodeType === 1 && n.classList.contains('row-detail')) return;
      text += ' ' + n.textContent;
    });
    // A chip is part of the text, so a licence chip is removed from that
    // string rather than skipped while assembling one.
    if (licenseWords.length) {
      card.querySelectorAll('.tag').forEach(chip => {
        const label = chip.textContent;
        if (label && saysOnly(label, licenseWords)) text = text.split(label).join(' ');
      });
    }
    // Separators are preserved and only a licence token is blanked, so a phrase
    // spanning two keywords still matches as it did.
    const tags = String(card.dataset.tags || '')
      .replace(/[^\s,]+/g, t => (saysOnly(t, licenseWords) ? '' : t));
    return (text + ' ' + tags).toLowerCase();
  }

  function anyShown(sel) {
    return Array.prototype.some.call(document.querySelectorAll(sel), c => c.style.display !== 'none');
  }

  function filterExtensions(q) {
    const bar = document.getElementById('ext-chips');
    const active = activeChips(bar);
    const facets = Object.keys(active);
    const filtering = !!q || facets.length > 0;
    applyAdapterVisibility(filtering);

    const cards = document.querySelectorAll('#extensions-section .ext-card');
    const hits = searchMatches(q, cards, window.MarketplaceSearch.EXTENSION_NOISE);
    let visible = 0;
    cards.forEach(c => {
      const hay = searchHaystack(c);
      let show = (!q || hits.has(c));
      if (show) {
        for (let i = 0; i < facets.length; i++) {
          const f = facets[i], vals = active[f];
          // `org` and `privacy` are exact-value facets: they match the card's
          // own dataset entry, not a substring of its prose. A privacy
          // substring match would make every card mentioning "private" answer
          // the Private chip.
          const ok = (f === 'org' || f === 'privacy') ? vals.indexOf(c.dataset[f] || '') !== -1 : vals.some(v => hay.indexOf(v) !== -1);
          if (!ok) { show = false; break; }
        }
      }
      c.style.display = show ? '' : 'none';
      if (show) visible++;
    });

    const featWrap = document.getElementById('ext-featured-wrap');
    if (featWrap && featWrap.dataset.empty !== '1' && document.querySelector('#ext-featured .ext-card')) {
      featWrap.hidden = !anyShown('#ext-featured .ext-card');
    }
    const primaryWrap = document.getElementById('ext-primary-wrap');
    if (primaryWrap && document.querySelector('#ext-primary .ext-card')) {
      primaryWrap.hidden = !anyShown('#ext-primary .ext-card');
    }
    const adaptHead = document.getElementById('ext-adapters-head');
    if (adaptHead) adaptHead.hidden = filtering && !anyShown('#ext-adapters .ext-card');
    const nr = document.getElementById('extensions-no-results');
    if (nr) nr.style.display = (visible === 0) ? '' : 'none';
    // The note says something only while a filter narrows the shelf; in the
    // browse state the tab already carries the total.
    setCount('ext-total-count', filtering ? (visible + ' of ' + cards.length + ' extensions') : '');
    // A row inside a hidden container measures zero and cannot be trimmed, so
    // re-run once every card's final display is set.
    trimTagRows();
  }

  async function loadRegistryExtensions() {
    try {
      const res = await fetch('registry.json', { cache: 'no-store' });
      if (!res.ok) throw new Error('registry fetch failed');
      const registry = await res.json();
      if (!Array.isArray(registry.extensions) || registry.extensions.length === 0) return;
      // Validated BEFORE anything is written to the DOM: renderExtensions()
      // empties the static grid as its last step, so a half-applied render has
      // already destroyed the fallback it would need to fall back to.
      if (!readableTiers(registry.extensions)) return;
      if (!readableAffiliations(registry.extensions, registry.institutions)) return;
      renderExtensions(registry.extensions, registry.institutions || {});
      if (activeShelf === 'extensions') runFilter();
    } catch (e) { /* keep the static no-JS fallback rows */ }
  }

  /* ─────────────────────────────────────────────────────────────
     Skills: static HTML, filtered and progressively disclosed
     ───────────────────────────────────────────────────────────── */
  function initSkills() {
    document.querySelectorAll('#skills-section .skill-card').forEach(card => {
      const tt = (card.querySelector('.skill-type') ? card.querySelector('.skill-type').textContent : '').toLowerCase();
      card._type = tt.indexOf('auto-applied') !== -1 ? 'auto' : 'user';
      let cat = card.dataset.cat;
      if (!cat) { const g = card.closest('[data-cat]'); cat = g ? g.dataset.cat : ''; }
      card._cat = cat || '';
      if (card._cat === 'biomedical') bioGlyph(card);
    });
    document.querySelectorAll('#skills-section .show-all-btn').forEach(btn => {
      btn.setAttribute('aria-expanded', 'false');
      btn.addEventListener('click', () => {
        const grid = document.getElementById(btn.dataset.target);
        if (!grid) return;
        const nowCollapsed = grid.classList.toggle('collapsed');
        btn.classList.toggle('open', !nowCollapsed);
        btn.setAttribute('aria-expanded', nowCollapsed ? 'false' : 'true');
        btn.querySelector('.lbl').textContent = nowCollapsed
          ? ('Show all ' + grid.querySelectorAll('.skill-card').length)
          : 'Show fewer';
        clampAll();
      });
    });
    const bar = document.getElementById('skill-chips');
    if (bar) bar.querySelectorAll('.fchip').forEach(btn => btn.addEventListener('click', () => onChipClick(bar, btn)));
  }

  function filterSkills(q) {
    const bar = document.getElementById('skill-chips');
    const active = activeChips(bar);
    const facets = Object.keys(active);
    const filtering = !!q || facets.length > 0;

    const cards = document.querySelectorAll('#skills-section .skill-card');
    const hits = searchMatches(q, cards, window.MarketplaceSearch.SKILL_NOISE);
    let visible = 0;
    cards.forEach(c => {
      let show = (!q || hits.has(c));
      if (show) {
        for (let i = 0; i < facets.length; i++) {
          const f = facets[i], vals = active[f];
          let ok = true;
          if (f === 'category') ok = vals.indexOf(c._cat) !== -1;
          else if (f === 'type') ok = vals.indexOf(c._type) !== -1;
          if (!ok) { show = false; break; }
        }
      }
      c.style.display = show ? '' : 'none';
      if (show) visible++;
    });

    // Group heads and collapse behaviour. A filtered group un-collapses, so
    // every row it kept is displayed.
    document.querySelectorAll('#skills-section .subsection-title').forEach(title => {
      let grid = title.nextElementSibling;
      while (grid && !(grid.classList && grid.classList.contains('skill-grid'))) grid = grid.nextElementSibling;
      if (!grid) return;
      let btn = grid.nextElementSibling;
      while (btn && !(btn.classList && btn.classList.contains('show-all-btn'))) btn = btn.nextElementSibling;
      const anyVisible = Array.prototype.some.call(grid.querySelectorAll('.skill-card'), c => c.style.display !== 'none');
      title.style.display = anyVisible ? '' : 'none';
      grid.style.display = anyVisible ? '' : 'none';
      if (btn) btn.style.display = anyVisible ? '' : 'none';
      if (filtering) {
        grid.classList.remove('collapsed');
        if (btn) btn.hidden = true;
      } else if (btn) {
        btn.hidden = false;
        btn.classList.remove('open');
        btn.setAttribute('aria-expanded', 'false');
        grid.classList.add('collapsed');
        btn.querySelector('.lbl').textContent = 'Show all ' + grid.querySelectorAll('.skill-card').length;
      }
    });

    // The featured strip repeats three grid rows. While a filter is on, the
    // grids already show every match, so the strip steps aside and each skill
    // appears once, matching the count.
    const fw = document.getElementById('skills-featured');
    if (fw) {
      const anyF = Array.prototype.some.call(fw.querySelectorAll('.skill-card'), c => c.style.display !== 'none');
      fw.hidden = filtering || !anyF;
    }
    const nr = document.getElementById('skills-no-results');
    if (nr) nr.style.display = (visible === 0) ? '' : 'none';
    const total = document.querySelectorAll('#skills-section .skill-grid .skill-card').length;
    const shownInGrids = Array.prototype.filter.call(document.querySelectorAll('#skills-section .skill-grid .skill-card'), c => c.style.display !== 'none').length;
    setCount('skills-total-count', filtering ? (shownInGrids + ' of ' + total + ' skills') : '');
  }

  /* ── Workflows: static HTML ─────────────────────────────────── */
  function filterWorkflows(q) {
    const cards = document.querySelectorAll('#workflows-section .wf-card');
    const hits = searchMatches(q, cards, window.MarketplaceSearch.WORKFLOW_NOISE);
    let visible = 0;
    cards.forEach(c => {
      const show = !q || hits.has(c);
      c.style.display = show ? '' : 'none';
      if (show) visible++;
    });
    const nr = document.getElementById('workflows-no-results');
    if (nr) nr.style.display = (q && visible === 0) ? '' : 'none';
  }

  /* ─────────────────────────────────────────────────────────────
     Opening a row in place
     ───────────────────────────────────────────────────────────── */
  const ROW = '.ext-card, .skill-card, .wf-card';

  function fileOf(href) { return String(href || '').split('?')[0].split('/').pop(); }
  function slugOf(href) { return fileOf(href).replace(/\.(brxt|zip)$/i, ''); }
  function nameOf(card) { const h = card.querySelector('h3'); return h ? h.textContent.trim() : ''; }

  // The registry id, the way build-registry.mjs derives it: the declared
  // data-registry-id, else the download file name without its extension.
  function extIdOf(card) {
    if (card.dataset.registryId) return card.dataset.registryId.trim();
    const dl = card.querySelector('.brxt-chip');
    return dl ? slugOf(dl.getAttribute('href')) : '';
  }

  function row(label, html) {
    return `<div class="detail-row"><dt>${label}</dt><dd>${html}</dd></div>`;
  }
  function list(items, cls) {
    return `<ul class="${cls}">${items.map(t => `<li>${escapeHtml(t)}</li>`).join('')}</ul>`;
  }

  // The one place the page says how a secret is entered. It shows only on a
  // row whose requirements name a secret (a key, token, password or
  // passcode), and the requirements text itself never repeats it.
  const SECRET_NOTE = '<p class="fine">Biorouter asks for these values in its own dialog when you install it, and keeps the secret ones in your system\'s credential store. Never paste a secret into a chat.</p>';
  function needsSecret(d) {
    return !!(d && d.needs && /secret|PASSWORD|PASSCODE|_KEY\b|TOKEN/.test(d.needs));
  }

  function extDetailHtml(card) {
    const id = extIdOf(card);
    const d = detailsFor(id);
    const name = escapeHtml(nameOf(card));
    const dl = card.querySelector('.brxt-chip');
    const href = dl ? dl.getAttribute('href') : '';
    const file = escapeHtml(fileOf(href));
    const gh = card.querySelector('.ext-gh-link');
    const rows = [];
    if (d && d.what) rows.push(row('What it does', `<p>${escapeHtml(d.what)}</p>`));
    if (card.dataset.privacy === 'private') {
      // A local model always qualifies (privacy/affiliation.rs compatible()),
      // so the refusal is named by tier, never as "hosted outside UCSF".
      rows.push(row('Privacy', '<p>Private. Public models cannot use it. When it answers in a chat, that chat becomes private and stays private, and public models are refused there from then on.</p>'));
    }
    if (d && d.asks && d.asks.length) rows.push(row('Try asking', list(d.asks, 'asks')));
    if (d && d.tools && d.tools.length) {
      let html = `<p class="tools">${d.tools.map(t => `<code>${escapeHtml(t)}</code>`).join('')}</p>`;
      if (d.toolsNote) html += `<p class="fine">${escapeHtml(d.toolsNote)}</p>`;
      if (d.skills && d.skills.length) {
        html += `<p class="fine">Comes with the ${d.skills.map(s => `<code>${escapeHtml(s)}</code>`).join(', ')} skill.</p>`;
      }
      rows.push(row('Main tools', html));
    }
    if (d && d.needs) rows.push(row('Requirements', `<p>${escapeHtml(d.needs)}</p>`));
    if (d && d.sources && d.sources.length) rows.push(row('Data sources', list(d.sources, 'src')));
    rows.push(row('Install', `<ul class="how">
      <li><span class="k">In the app</span><span>Open Extensions, choose Browse extensions, then Add beside ${name}.</span></li>
      <li><span class="k">From a file</span><span>Download <a href="${escapeHtml(href)}">${file}</a>. In Extensions, choose Add extension and drop the file in.</span></li>
      <li><span class="k">At a terminal</span><span><code>biorouter extension install ./${file}</code></span></li>
    </ul>${needsSecret(d) ? SECRET_NOTE : ''}`));
    const links = [];
    if (gh) links.push(`<a class="text-link" href="${escapeHtml(gh.getAttribute('href'))}" target="_blank" rel="noopener">Source on GitHub <span aria-hidden="true">↗</span></a>`);
    ((d && d.links) || []).forEach(l => links.push(`<a class="text-link" href="${escapeHtml(l.href)}" target="_blank" rel="noopener">${escapeHtml(l.label)} <span aria-hidden="true">↗</span></a>`));
    return `<dl class="detail-list">${rows.join('')}</dl>` + (links.length ? `<p class="detail-links">${links.join('')}</p>` : '');
  }

  // What the row's type line means, in plain words. The line is registry
  // data ("User-invocable · /r-scripting", "14 skills · auto-applied"), so it
  // is read here, never rewritten.
  function skillTypeHtml(card) {
    const line = card.querySelector('.skill-type');
    const parts = String(line ? line.textContent : '').split('·').map(t => t.trim());
    const slug = parts.find(t => t.charAt(0) === '/');
    const pack = /^(\d+)\s+skills?$/i.exec(parts[0] || '');
    if (pack && +pack[1] > 1) return `<p>A package of ${escapeHtml(pack[1])} skills. The agent loads each one on its own when a task needs it.</p>`;
    if (/^user-invocable$/i.test(parts[0] || '') && slug) {
      return `<p>User-invocable: you start it by name. Type <code>${escapeHtml(slug)}</code> in the chat box and pick it from the list. The agent can also load it when a task needs it.</p>`;
    }
    if (/^auto-applied$/i.test(parts[0] || '')) return '<p>Auto-applied. The agent loads it on its own when a task needs it, so you do not need to name it.</p>';
    return '';
  }

  function skillDetailHtml(card) {
    const name = escapeHtml(nameOf(card));
    const dl = card.querySelector('.skill-dl-btn');
    const href = dl ? dl.getAttribute('href') : '';
    const file = escapeHtml(fileOf(href));
    const type = skillTypeHtml(card);
    return `<dl class="detail-list">${type ? row('How it runs', type) : ''}${row('Install', `<ul class="how">
      <li><span class="k">In the app</span><span>Open Skills, choose Browse skills, select ${name} and choose Install.</span></li>
      <li><span class="k">From a file</span><span>Download <a href="${escapeHtml(href)}">${file}</a>. In Skills, choose Add skill and drop the file in.</span></li>
    </ul>`)}</dl>`;
  }

  function setOpen(card, open) {
    const btn = card.querySelector(':scope > .row-toggle');
    let panel = card.querySelector(':scope > .row-detail');
    if (open && !panel) {
      panel = document.createElement('div');
      panel.className = 'row-detail';
      panel.id = 'detail-' + Math.random().toString(36).slice(2, 9);
      panel.innerHTML = card.classList.contains('skill-card') ? skillDetailHtml(card) : extDetailHtml(card);
      card.appendChild(panel);
    }
    if (panel) {
      panel.hidden = !open;
      if (btn) btn.setAttribute('aria-controls', panel.id);
    }
    card.classList.toggle('open', open);
    if (btn) btn.setAttribute('aria-expanded', open ? 'true' : 'false');
    if (!open) card.querySelectorAll(':scope > .ext-desc, :scope > .skill-desc, :scope > .wf-desc').forEach(clampDesc);
  }

  function ensureToggles(scope) {
    (scope || document).querySelectorAll(ROW).forEach(card => {
      if (card.querySelector(':scope > .row-toggle')) return;
      card.appendChild(toggleButton(nameOf(card)));
    });
  }

  function onRowClick(e) {
    const card = e.target.closest(ROW);
    if (!card || !card.closest('#main')) return;
    const toggle = e.target.closest('.row-toggle');
    if (!toggle) {
      // Links, buttons, the open panel and a text selection keep their own meaning.
      if (e.target.closest('a, button, input, .row-detail')) return;
      const sel = window.getSelection && window.getSelection();
      if (sel && String(sel).trim() && card.contains(sel.anchorNode)) return;
    }
    setOpen(card, !card.classList.contains('open'));
  }

  /* ── Two lines of description on a closed row ────────────────
     The text is split, never cut: the paragraph's textContent stays exactly
     the description, which is what the search and the facets read, and an
     open row shows the rest. A CSS line clamp looks the same but leaves the
     hidden lines laid out under the paragraph, on top of the tag row. */
  const CLAMP_LINES = 2;
  const CLAMP_SEL = '#extensions-section .ext-desc, #workflows-section .wf-desc, #skills-section .skill-desc';

  function clampDesc(p) {
    const card = p.closest(ROW);
    // An open row shows everything; it is measured again when it closes.
    if (!card || card.classList.contains('open')) return;
    const width = p.clientWidth;
    if (!width) return; // not laid out: a hidden shelf, group or row
    const text = p.textContent;
    const key = width + '|' + text;
    if (p._clampKey === key) return;
    p._clampKey = key;
    // Only a plain text paragraph, or one this function split before.
    for (const child of p.children) {
      if (!child.classList.contains('d-lead') && !child.classList.contains('d-rest')) return;
    }
    p.classList.remove('clamped');
    p.textContent = text;
    const node = p.firstChild;
    if (!node) return;
    const lineHeight = parseFloat(getComputedStyle(p).lineHeight) || 23;
    const box = () => p.getBoundingClientRect().height;
    if (box() <= lineHeight * CLAMP_LINES + 1) return;
    // The last character that still ends on line two.
    const bottom = p.getBoundingClientRect().top + lineHeight * CLAMP_LINES + 1;
    const range = document.createRange();
    const fits = k => {
      range.setStart(node, 0);
      range.setEnd(node, k);
      const rects = range.getClientRects();
      return rects.length === 0 || rects[rects.length - 1].bottom <= bottom;
    };
    let lo = 0, hi = text.length;
    while (hi - lo > 1) { const mid = (lo + hi) >> 1; if (fits(mid)) lo = mid; else hi = mid; }
    // Split at a space, backing off a word at a time until the ellipsis fits.
    const lead = document.createElement('span');
    const rest = document.createElement('span');
    lead.className = 'd-lead';
    rest.className = 'd-rest';
    let cut = text.lastIndexOf(' ', lo);
    for (let tries = 0; tries < 8 && cut > 0; tries++) {
      // The ellipsis follows a word, never a comma or a full stop.
      let end = cut;
      while (end > 0 && /[\s,;:.]/.test(text.charAt(end - 1))) end--;
      if (end === 0) break;
      lead.textContent = text.slice(0, end);
      rest.textContent = text.slice(end);
      if (!lead.parentNode) { p.replaceChildren(lead, rest); p.classList.add('clamped'); }
      if (box() <= lineHeight * CLAMP_LINES + 1) return;
      cut = text.lastIndexOf(' ', cut - 1);
    }
    // Nothing fitted: show the whole text rather than a broken one.
    p.classList.remove('clamped');
    p.textContent = text;
  }

  function clampAll() {
    document.querySelectorAll(CLAMP_SEL).forEach(clampDesc);
  }

  /* ── Counts driven from actual rendered cards ──────────────── */
  function setText(id, txt) { const el = document.getElementById(id); if (el) el.textContent = txt; }
  // A shelf's quiet count note, hidden while it has nothing to say.
  function setCount(id, txt) {
    const el = document.getElementById(id);
    if (!el) return;
    el.textContent = txt;
    if (el.parentElement) el.parentElement.hidden = !txt;
  }
  function updateCounts() {
    const extTotal = document.querySelectorAll('#ext-featured .ext-card, #ext-primary .ext-card, #ext-adapters .ext-card').length
      || document.querySelectorAll('#extensions-grid .ext-card').length;
    const core = document.querySelectorAll('#core-skill-grid .skill-card').length;
    const dev = document.querySelectorAll('#dev-skill-grid .skill-card').length;
    const bio = document.querySelectorAll('#bio-skill-grid .skill-card').length;
    const skillTotal = core + dev + bio;
    const wfTotal = document.querySelectorAll('#workflows-section .wf-card').length;
    setText('tab-count-extensions', extTotal);
    setText('tab-count-skills', skillTotal);
    setText('tab-count-workflows', wfTotal);
    setText('count-core', core + ' skills');
    setText('count-dev', dev + ' skills');
    setText('count-bio', bio + ' categories');
  }

  /* ── Init ──────────────────────────────────────────────────── */
  (function init() {
    initSkills();
    ensureToggles();
    document.addEventListener('click', onRowClick);
    document.querySelectorAll('.baam-tab').forEach(t => t.addEventListener('click', () => setShelf(t.dataset.shelf, true)));
    // Arrow keys move between the shelf tabs, as a tablist should.
    const tablist = document.querySelector('.baam-tabs');
    if (tablist) tablist.addEventListener('keydown', e => {
      const tabs = [...tablist.querySelectorAll('.baam-tab')];
      const i = tabs.indexOf(document.activeElement);
      if (i === -1) return;
      let next = -1;
      if (e.key === 'ArrowRight') next = (i + 1) % tabs.length;
      else if (e.key === 'ArrowLeft') next = (i - 1 + tabs.length) % tabs.length;
      else if (e.key === 'Home') next = 0;
      else if (e.key === 'End') next = tabs.length - 1;
      if (next === -1) return;
      e.preventDefault();
      tabs[next].focus();
      setShelf(tabs[next].dataset.shelf, true);
    });
    const tgl = document.getElementById('ext-adapters-toggle');
    if (tgl) tgl.addEventListener('click', () => { adaptersManualOpen = !adaptersManualOpen; applyAdapterVisibility(false); trimTagRows(); clampAll(); });
    window.addEventListener('hashchange', () => { const s = shelfFromHash(); if (s) setShelf(s, false); });
    window.addEventListener('resize', trimTagRows);
    let clampFrame = 0;
    window.addEventListener('resize', () => {
      cancelAnimationFrame(clampFrame);
      clampFrame = requestAnimationFrame(clampAll);
    });
    updateCounts();
    setShelf(shelfFromHash() || 'extensions', false);
    loadRegistryExtensions();
  })();
