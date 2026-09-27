/* ───────────────────────────────────────────────────────────────
   UCSF Biorouter — App UI mockup renderer
   Builds hand-coded, vector recreations of the Biorouter desktop app
   screens and mounts them into [data-screen] containers, replacing the
   old low-resolution screenshots. Pure DOM, so it zooms losslessly and
   reads at the website's own text size.
   Mirrors the real app: sidebar order from AppSidebar.tsx, screen
   layouts from Hub / Chat / Sessions / Workflows / Extensions / Skills /
   Knowledge / Scheduler / Provider configuration.
   ─────────────────────────────────────────────────────────────── */
(function () {
  'use strict';

  /* Exact icon paths extracted from the app's lucide-react v0.562 set
     (ui/desktop/src/components/icons/app-icons.tsx, strokeWidth 1.5), plus
     the app's hand-crafted KnowledgeIcon / Attach / Send. Using the real
     glyphs keeps the website mockups consistent with the live app. */
  var I = {
    home: '<path d="M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8"/><path d="M3 10a2 2 0 0 1 .709-1.528l7-6a2 2 0 0 1 2.582 0l7 6A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
    chat: '<path d="M22 17a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 21.286V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2z"/>',
    history: '<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l4 2"/>',
    workflows: '<rect width="8" height="8" x="3" y="3" rx="2"/><path d="M7 11v4a2 2 0 0 0 2 2h4"/><rect width="8" height="8" x="13" y="13" rx="2"/>',
    scheduler: '<path d="M12 6v6l4 2"/><circle cx="12" cy="12" r="10"/>',
    extensions: '<path d="M15.39 4.39a1 1 0 0 0 1.68-.474 2.5 2.5 0 1 1 3.014 3.015 1 1 0 0 0-.474 1.68l1.683 1.682a2.414 2.414 0 0 1 0 3.414L19.61 15.39a1 1 0 0 1-1.68-.474 2.5 2.5 0 1 0-3.014 3.015 1 1 0 0 1 .474 1.68l-1.683 1.682a2.414 2.414 0 0 1-3.414 0L8.61 19.61a1 1 0 0 0-1.68.474 2.5 2.5 0 1 1-3.014-3.015 1 1 0 0 0 .474-1.68l-1.683-1.682a2.414 2.414 0 0 1 0-3.414L4.39 8.61a1 1 0 0 1 1.68.474 2.5 2.5 0 1 0 3.014-3.015 1 1 0 0 1-.474-1.68l1.683-1.682a2.414 2.414 0 0 1 3.414 0z"/>',
    skills: '<path d="M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83z"/><path d="M2 12a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 12"/><path d="M2 17a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 17"/>',
    knowledge: '<circle cx="12" cy="12" r="3"/><circle cx="5" cy="6" r="1.6"/><circle cx="19" cy="6" r="1.6"/><circle cx="6" cy="18" r="1.6"/><circle cx="18" cy="18" r="1.6"/><path d="M10 10.5L6 7M14 10.5l4-3.5M10 14l-4 3M14 14l4 3"/>',
    apps: '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="M2 8h20"/><path d="M6 4v4"/><path d="M10 4v4"/>',
    settings: '<path d="M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915"/><circle cx="12" cy="12" r="3"/>',
    plus: '<path d="M5 12h14"/><path d="M12 5v14"/>',
    panel: '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/>',
    cal: '<path d="M8 2v4"/><path d="M16 2v4"/><rect width="18" height="18" x="3" y="4" rx="2"/><path d="M3 10h18"/>',
    chevR: '<path d="m9 18 6-6-6-6"/>',
    chevD: '<path d="m6 9 6 6 6-6"/>',
    db: '<ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5V19A9 3 0 0 0 21 19V5"/><path d="M3 12A9 3 0 0 0 21 12"/>',
    search: '<path d="m21 21-4.34-4.34"/><circle cx="11" cy="11" r="8"/>',
    refresh: '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>',
    pause: '<rect x="14" y="4" width="4" height="16" rx="1"/><rect x="6" y="4" width="4" height="16" rx="1"/>',
    upload: '<path d="M12 3v12"/><path d="m17 8-5-5-5 5"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>',
    download: '<path d="M12 15V3"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/>',
    play: '<path d="M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z"/>',
    term: '<path d="M12 19h8"/><path d="m4 17 6-6-6-6"/>',
    ext: '<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
    edit: '<path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.375 2.625a1 1 0 0 1 3 3l-9.013 9.014a2 2 0 0 1-.853.505l-2.873.84a.5.5 0 0 1-.62-.62l.84-2.873a2 2 0 0 1 .506-.852z"/>',
    share: '<circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><line x1="8.59" x2="15.42" y1="13.51" y2="17.49"/><line x1="15.41" x2="8.59" y1="6.51" y2="10.49"/>',
    clock: '<path d="M12 6v6l4 2"/><circle cx="12" cy="12" r="10"/>',
    trash: '<path d="M10 11v6"/><path d="M14 11v6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M3 6h18"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>',
    msg: '<path d="M22 17a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 21.286V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2z"/><path d="M7 11h10"/><path d="M7 15h6"/><path d="M7 7h8"/>',
    target: '<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="6"/><circle cx="12" cy="12" r="2"/>',
    folder: '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>',
    folderDot: '<path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z"/><circle cx="12" cy="13" r="1"/>',
    book: '<path d="M12 7v14"/><path d="M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3z"/>',
    dollar: '<line x1="12" x2="12" y1="2" y2="22"/><path d="M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6"/>',
    brain: '<path d="M12 18V5"/><path d="M15 13a4.17 4.17 0 0 1-3-4 4.17 4.17 0 0 1-3 4"/><path d="M17.598 6.5A3 3 0 1 0 12 5a3 3 0 1 0-5.598 1.5"/><path d="M17.997 5.125a4 4 0 0 1 2.526 5.77"/><path d="M18 18a4 4 0 0 0 2-7.464"/><path d="M19.967 17.483A4 4 0 1 1 12 18a4 4 0 1 1-7.967-.517"/><path d="M6 18a4 4 0 0 1-2-7.464"/><path d="M6.003 5.125a4 4 0 0 0-2.526 5.77"/>',
    check: '<path d="M20 6 9 17l-5-5"/>',
    back: '<path d="m12 19-7-7 7-7"/><path d="M19 12H5"/>',
    clipboard: '<rect width="8" height="4" x="8" y="2" rx="1" ry="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/>',
    wmin: '<path d="M5 12h14"/>',
    minimize: '<path d="m14 10 7-7"/><path d="M20 10h-6V4"/><path d="m3 21 7-7"/><path d="M4 14h6v6"/>',
    wexp: '<path d="M15 3h6v6"/><path d="m21 3-7 7"/><path d="m3 21 7-7"/><path d="M9 21H3v-6"/>',
    wx: '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
    folderopen: '<path d="m6 14 1.45-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.55 6a2 2 0 0 1-1.94 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.93a2 2 0 0 1 1.66.9l.82 1.2a2 2 0 0 0 1.66.9H18a2 2 0 0 1 2 2v2"/>',
    /* Filled custom icons (Attach paperclip, Send paper-plane) — rendered solid */
    attach: '<path fill-rule="evenodd" clip-rule="evenodd" d="M9.896 3.95a6.412 6.412 0 0 1 9.208 0c2.528 2.585 2.528 6.77 0 9.355l-7.13 7.295a4.608 4.608 0 0 1-6.615 0c-1.812-1.854-1.812-4.85 0-6.703l7.13-7.295a2.804 2.804 0 0 1 4.023 0 2.907 2.907 0 0 1 0 4.05l-7.13 7.296a1 1 0 0 1-1.43-1.399l7.13-7.294a.907.907 0 0 0 0-1.255.804.804 0 0 0-1.163 0l-7.13 7.295a2.813 2.813 0 0 0 0 3.907 2.608 2.608 0 0 0 3.755 0l7.13-7.295c1.768-1.809 1.768-4.75 0-6.56a4.412 4.412 0 0 0-6.348 0l-.972.995a1 1 0 0 1-1.43-1.398l.972-.995Z"/>',
    send: '<path fill-rule="evenodd" clip-rule="evenodd" d="M20.707 3.293a1 1 0 0 1 .25.994l-5.4 18a1 1 0 0 1-1.886.084l-3.44-8.602-8.602-3.44a1 1 0 0 1 .084-1.887l18-5.4a1 1 0 0 1 .994.25Zm-8.534 9.948 2.292 5.729L18.509 5.49 5.03 9.535l5.73 2.292 1.333-1.334a1 1 0 0 1 1.414 1.414l-1.334 1.334Z"/>',
  };

  /* Icons drawn solid (fill) rather than stroked. */
  var FILLED = { attach: 1, send: 1, play: 1 };

  function svg(path, cls) {
    var c = (cls ? cls + ' ' : '');
    // detect filled icons by content
    return '<svg viewBox="0 0 24 24" class="' + c + '">' + path + '</svg>';
  }
  function svgI(key, cls) {
    var c = (FILLED[key] ? 'bw-fill ' : '') + (cls || '');
    return '<svg viewBox="0 0 24 24" class="' + c.trim() + '">' + I[key] + '</svg>';
  }

  /* Sidebar nav definition. Labels and order mirror AppSidebar.tsx:
     primaryItems (Home, New chat), then the six componentItems that sit behind
     the `Components` disclosure, then settingsItem pinned to the footer. The
     mockup shows the six expanded, which is one click away in the app.

     There is no History row: History is reached from the Recents list, so the
     history screen below passes a key that matches nothing and no row lights
     up, the same as the app. */
  var NAV = [
    { k: 'home', label: 'Home', icon: 'home' },
    { sep: true },
    { k: 'newchat', label: 'New chat', icon: 'plus' },
    { sep: true },
    { k: 'workflows', label: 'Workflows', icon: 'workflows' },
    { k: 'scheduler', label: 'Scheduler', icon: 'scheduler' },
    { k: 'extensions', label: 'Extensions', icon: 'extensions' },
    { k: 'skills', label: 'Skills', icon: 'skills' },
    { k: 'knowledge', label: 'Knowledge', icon: 'knowledge' },
    { k: 'apps', label: 'Built apps', icon: 'apps' },
    { sep: true },
    { k: 'settings', label: 'Settings', icon: 'settings' },
  ];

  function sidebar(active) {
    var items = NAV.map(function (n) {
      if (n.sep) return '<div class="sep"></div>';
      var on = n.k === active ? ' active' : '';
      return '<div class="item' + on + '">' +
        '<svg viewBox="0 0 24 24">' + I[n.icon] + '</svg><span>' + n.label + '</span></div>';
    }).join('');
    return '<div class="bw-side"><div class="bw-nav">' + items + '</div>' +
      '<div class="bw-side-foot"><img src="icon.svg" alt=""><b>Biorouter</b><span class="status"></span></div></div>';
  }

  /* Title bar. opts: {icons:bool, center:html, right:html} */
  function bar(opts) {
    opts = opts || {};
    var lights = '<div class="bw-lights"><i class="r"></i><i class="y"></i><i class="g"></i></div>';
    var icons = opts.icons === false ? '' :
      '<div class="bw-bar-icons">' + svg(I.plus) + svg(I.panel) + '</div>';
    var center = opts.center ? '<div class="bw-bar-center">' + opts.center + '</div>' : '<div class="bw-bar-spacer"></div>';
    var right = opts.right ? '<div class="bw-bar-meta">' + opts.right + '</div>' : '';
    return '<div class="bw-bar">' + lights + icons + center + right + '</div>';
  }

  /* Standard window: title bar + sidebar + main */
  function win(active, barOpts, main) {
    return '<div class="bw">' + bar(barOpts) + '<div class="bw-body">' + sidebar(active) +
      '<div class="bw-main">' + main + '</div></div></div>';
  }

  /* Composer block. Toolbar order mirrors ChatInput.tsx:
     DirSwitcher (FolderDot), Attach, Extensions (Puzzle), Skills (Layers),
     Knowledge (BookOpen), Cost (DollarSign), Model (Brain), mode label. */
  function composer(stop) {
    var right = stop
      ? '<span class="bw-stop"><i></i></span>'
      : '<span class="bw-send">' + svgI('send') + 'Send</span>';
    return '<div class="bw-composer">' +
      '<div class="ph">⌘↑ / ⌘↓ to navigate messages<span class="caret"></span></div>' +
      '<div class="bw-toolbar">' +
      '<span class="chip">' + svgI('folderDot') + 'Users/wgu/Desktop/</span>' +
      '<span class="chip">' + svgI('attach') + '10</span>' +
      '<span class="chip">' + svgI('extensions') + '8</span>' +
      '<span class="chip">' + svgI('skills') + '5</span>' +
      '<span class="chip">' + svgI('book') + '6 KBs visible</span>' +
      '<span class="chip">' + svgI('dollar') + '0.0000</span>' +
      '<span class="chip">' + svgI('brain') + 'gpt-6-sol</span>' +
      '<span class="grow"></span>' + right + '</div></div>';
  }

  /* Compact composer used inside a split pane. When a chat is narrower than a
     full window the app (ChatInput compactPicker) keeps only DirSwitcher /
     Attach / Extensions / Skills visible and folds the rest (cost, model,
     mode) behind a chevron. */
  function composerCompact() {
    return '<div class="bw-composer">' +
      '<div class="ph">⌘↑ / ⌘↓ to navigate messages<span class="caret"></span></div>' +
      '<div class="bw-toolbar">' +
      '<span class="chip">' + svgI('folderDot') + 'Users/wgu/Desktop/</span>' +
      '<span class="chip">' + svgI('attach') + '10</span>' +
      '<span class="chip">' + svgI('extensions') + '8</span>' +
      '<span class="chip">' + svgI('skills') + '5</span>' +
      '<span class="chip">' + svg(I.chevR) + '</span>' +
      '<span class="grow"></span>' +
      '<span class="bw-send">' + svgI('send') + 'Send</span></div></div>';
  }

  /* ─── Screen builders ──────────────────────────────────────── */
  var SCREENS = {};

  SCREENS.home = function () {
    function stat(label, big, a, b) {
      return '<div class="bw-stat"><div class="lbl">' + label + '</div><div class="nums">' +
        '<div><div class="n">' + big + '</div><div class="sub">Total</div></div>' +
        '<div><div class="n sm">' + a + '</div><div class="sub">Past 30 days</div></div>' +
        '<div><div class="n sm">' + b + '</div><div class="sub">Past 7 days</div></div></div></div>';
    }
    function row(t, d) {
      return '<div class="bw-list-row bw-stagger">' + svg(I.chat) + '<span class="t">' + t + '</span>' +
        '<span class="d">' + d + '</span></div>';
    }
    var main = '<div class="bw-scroll bw-home">' +
      '<div class="tiny">UCSF Biorouter</div>' +
      '<h1>Which medical mystery might the knowledge graph help solve today?</h1>' +
      '<div class="bw-stats">' + stat('Sessions', '1,122', '556', '111') +
      stat('Tokens', '239.2M', '56.52M', '7.79M') + '</div>' +
      '<div class="bw-recent-head"><span class="lbl">Recent chats</span><a>See all</a></div>' +
      row('Remove codegraph indices folder', '06/03/2026') +
      row('New Session', '06/03/2026') +
      row('New Session', '06/03/2026') +
      '</div>' + composer(false);
    return win('home', {}, main);
  };

  SCREENS.chat = function () {
    var tools = [
      'Cdw-search Diagnoses By Code', 'Cdw-search Medications By Code',
      'Cdw-search Conditions By Code', 'Cdw-search Medications By Code',
    ];
    var rows = tools.map(function (t) {
      return '<div class="bw-tool bw-stagger"><span class="ic">' + svg(I.db) + '</span>' +
        '<span class="t"><b>' + t + '</b> <span class="arg">search_term, row_limit</span></span>' +
        '<span class="chev">' + svg(I.chevR) + '</span></div>';
    }).join('');
    rows += '<div class="bw-tool bw-stagger"><span class="ic">' + svg(I.db) + '</span>' +
      '<span class="t"><b>Cdw-describe Table</b> <span class="arg">table_name: deid_uf.MedicationOrderFact</span></span>' +
      '<span class="chev">' + svg(I.chevR) + '</span></div>';
    rows += '<div class="bw-tool bw-stagger"><span class="ic">' + svg(I.search) + '</span>' +
      '<span class="t"><b>Search Available Extensions</b></span>' +
      '<span class="chev">' + svg(I.chevR) + '</span></div>';
    var main = '<div class="bw-scroll" style="overflow:hidden">' +
      '<div class="bw-chat-title">New Session</div>' +
      '<div class="bw-bubble">How many patients with type 2 diabetes are currently receiving statin therapy?</div>' +
      '<div class="bw-bubble-time">4:52 PM</div>' + rows +
      '<div class="bw-working"><span class="code">&lt;/&gt;</span> <span class="shim">biorouter is working on it…</span></div>' +
      '</div>' + composer(true);
    return win('newchat', {}, main);
  };

  SCREENS.tabs = function () {
    /* Mirrors src/components/chatGroups: a tab strip per group (ChatTabStrip)
       and a resizable splitter between two leaf groups (ChatGroupSplitter).
       Each tab is an independent session with its own history and model; a
       running tab shows the small activity dot. */
    function tab(name, active, busy) {
      return '<div class="bw-tab' + (active ? ' active' : '') + '">' +
        '<span class="ic">' + svg(I.msg) + '</span>' +
        '<span class="nm">' + name + '</span>' +
        (busy ? '<span class="bw-tab-dot busy"></span>'
              : '<span class="bw-tab-dot">' + svg(I.wx) + '</span>') +
        '</div>';
    }
    function strip(tabs) {
      return '<div class="bw-tabs">' + tabs +
        '<span class="bw-tab-add">' + svg(I.plus) + '</span></div>';
    }

    /* Left pane: a live agent turn. */
    var leftTools = ['Cdw-search Diagnoses By Code', 'Cdw-search Medications By Code']
      .map(function (t) {
        return '<div class="bw-tool bw-stagger"><span class="ic">' + svg(I.db) + '</span>' +
          '<span class="t"><b>' + t + '</b> <span class="arg">search_term, row_limit</span></span>' +
          '<span class="chev">' + svg(I.chevR) + '</span></div>';
      }).join('');
    var leftBody = '<div class="bw-scroll" style="overflow:hidden">' +
      '<div class="bw-bubble">How many patients with type 2 diabetes are currently receiving statin therapy?</div>' +
      '<div class="bw-bubble-time">4:52 PM</div>' + leftTools +
      '<div class="bw-working"><span class="code">&lt;/&gt;</span> ' +
      '<span class="shim">biorouter is working on it…</span></div>' +
      '</div>' + composerCompact();
    var left = '<div class="bw-pane">' +
      strip(tab('OMOP statin cohort', true, true) + tab('GWAS · Type 2 Diabetes', false, false) +
            tab('EPAS1 literature scan', false, true)) +
      leftBody + '</div>';

    /* Right pane: a second group, running its own session in parallel. */
    var rightTools = ['Spoke-run Cypher Query', 'Developer-text Editor']
      .map(function (t) {
        return '<div class="bw-tool bw-stagger"><span class="ic">' + svg(I.db) + '</span>' +
          '<span class="t"><b>' + t + '</b></span>' +
          '<span class="chev">' + svg(I.chevR) + '</span></div>';
      }).join('');
    var rightBody = '<div class="bw-scroll" style="overflow:hidden">' +
      '<div class="bw-bubble">Which genes are linked to multiple sclerosis in SPOKE?</div>' +
      '<div class="bw-bubble-time">4:53 PM</div>' + rightTools +
      '<div class="bw-working"><span class="code">&lt;/&gt;</span> ' +
      '<span class="shim">biorouter is working on it…</span></div>' +
      '</div>' + composerCompact();
    var right = '<div class="bw-pane">' +
      strip(tab('SPOKE MS query', true, true) + tab('mtcars Shiny app', false, false)) +
      rightBody + '</div>';

    var main = '<div class="bw-split">' + left +
      '<div class="bw-splitter"></div>' + right + '</div>';
    return win('newchat', {}, main);
  };

  SCREENS.history = function () {
    function hrow(t, time, msgs, toks) {
      return '<div class="bw-hrow bw-stagger"><div class="body"><div class="t">' + t + '</div>' +
        '<div class="m"><span>' + svg(I.cal) + time + '</span><span>' + svg(I.folder) + '/Users/wgu/Desktop</span></div></div>' +
        '<div class="meta"><span>' + svg(I.msg) + msgs + '</span><span>' + svg(I.target) + toks + '</span></div></div>';
    }
    var main = '<div class="bw-scroll">' +
      '<div class="bw-headrow"><div class="bw-head"><h2>Chat history</h2>' +
      '<p>View and search your past conversations with Biorouter. ⌘F to search.</p></div>' +
      '<span class="bw-btn soft">' + svg(I.upload) + 'Import Session</span></div>' +
      '<div class="bw-sectionrule"></div>' +
      '<div class="bw-day">Today</div>' +
      hrow('New Session', '4:52 PM', '1', '16,503', '10') +
      hrow('Remove codegraph indices folder', '1:14 PM', '87', '41,834', '9') +
      hrow('New Session', '1:12 PM', '12', '3,356', '2') +
      hrow('GWAS on Type 2 Diabetes', '1:07 PM', '27', '102,443', '10') +
      hrow('EHR of Patients with Multiple Sclerosis', '1:07 PM', '4', '25,478', '10') +
      hrow('Functional Significance of the EPAS1 Gene', '1:06 PM', '20', '49,765', '10') +
      '<div class="bw-day">Monday, June 1</div>' +
      hrow('MS genes SPOKE query', '7:21 PM', '18', '29,652', '10') +
      '</div>';
    return win('history', {}, main);
  };

  SCREENS.workflows = function () {
    var act = function (icon, cls) {
      return '<span class="a ' + (cls || '') + '">' + svgI(icon) + '</span>';
    };
    var main = '<div class="bw-scroll">' +
      '<div class="bw-head"><h2>Workflows</h2>' +
      '<p>View and manage your saved workflows to quickly start new sessions with predefined configurations. ⌘F to search.</p>' +
      '<div class="bw-head-actions"><span class="bw-btn dark">' + svg(I.plus) + 'Create Workflow</span>' +
      '<span class="bw-btn soft">' + svg(I.upload) + 'Import Workflow</span></div></div>' +
      '<div class="bw-sectionrule"></div>' +
      '<div class="bw-wf bw-stagger"><div class="body">' +
      '<h4>Scientific Article Writing and Reference Validator</h4>' +
      '<p>A workflow for converting informal or plain-language content into formally structured scientific articles, and validating all cited references against live academic databases.</p>' +
      '<div class="date">' + svg(I.cal) + ' 5/12/2026</div></div>' +
      '<div class="bw-actions">' + act('term') + act('play', 'play') + act('ext') + act('edit') +
      act('share') + act('clock') + act('trash', 'del') + '</div></div>' +
      '</div>';
    return win('workflows', {}, main);
  };

  SCREENS.extensions = function () {
    function erow(name, desc, on) {
      return '<div class="bw-erow bw-stagger"><div class="body"><h4>' + name + '</h4><p>' + desc + '</p></div>' +
        '<span class="bw-switch' + (on ? ' on' : '') + '"></span></div>';
    }
    var main = '<div class="bw-scroll">' +
      '<div class="bw-head"><h2>Extensions</h2>' +
      '<p>MCP extensions expand Biorouter’s capabilities with Prompts, Resources, and Tools. Enable extension apps at all-new costs. ⌘F to search.</p>' +
      '<div class="bw-head-actions"><span class="bw-btn dark">' + svg(I.plus) + 'Add Extension</span>' +
      '<span class="bw-btn soft">' + svg(I.ext) + 'Browse Extensions</span>' +
      '<span class="bw-btn soft">' + svg(I.plus) + 'Add Custom Extension</span></div></div>' +
      '<div class="bw-sectionrule"></div>' +
      '<div class="bw-grp" style="margin:14px 0 2px"><i class="inst"></i>Built-in extensions (7)</div>' +
      erow('Developer', 'General development tools useful for software engineering.', true) +
      erow('Biorouter Copilot', 'Native desktop observation and control after task approval.', true) +
      erow('Web &amp; Documents', 'Read web pages and work with spreadsheets, documents, and PDFs.', true) +
      erow('Auto Visualiser', 'Data visualization and UI generation tools.', true) +
      erow('Memory', 'Teach Biorouter your preferences as you go.', true) +
      erow('Knowledge', 'Personal, LLM-maintained knowledge bases backed by markdown folders and git history.', true) +
      erow('Agent Drafter', 'Build interactive artifacts and export them as standalone projects.', true) +
      '</div>';
    return win('extensions', {}, main);
  };

  SCREENS.skills = function () {
    function tags(arr) {
      return '<div class="tags">' + arr.map(function (t) { return '<span class="tg">' + t + '</span>'; }).join('') + '</div>';
    }
    function skill(name, count, arr) {
      return '<div class="bw-skill bw-stagger"><div class="top"><span class="nm">' + name + '</span>' +
        '<span class="ct">' + count + '</span><span class="chev">' + svg(I.chevD) + '</span></div>' + tags(arr) + '</div>';
    }
    var main = '<div class="bw-scroll">' +
      '<div class="bw-head"><h2>Skills</h2>' +
      '<p>Reusable instruction sets that guide Biorouter’s behavior. ⌘F to search.</p>' +
      '<div class="bw-head-actions"><span class="bw-btn dark">' + svg(I.plus) + 'Add Skill</span>' +
      '<span class="bw-btn soft">' + svg(I.ext) + 'Browse Skills</span>' +
      '<span class="bw-btn soft">' + svg(I.plus) + 'Add Custom Skill</span></div></div>' +
      '<div class="bw-sectionrule"></div>' +
      '<div class="bw-grp" style="margin:14px 0 2px"><i class="inst"></i>Biorouter skills (62)</div>' +
      skill('clinical-databases', '13 skills', ['clinical-databases', 'allele-frequency', 'clinical-trials', 'drug-interactions', 'gene-disease', 'pathway-enrichment', 'variant-annotation', 'literature-search', 'omop-query']) +
      skill('single-cell', '9 skills', ['single-cell', 'qc-filtering', 'normalization', 'clustering', 'cell-typing', 'trajectory', 'differential-expression']) +
      skill('data-visualization', '6 skills', ['data-visualization', 'ggplot', 'volcano-plot', 'heatmap', 'survival-curve', 'forest-plot']) +
      skill('variant-calling', '7 skills', ['variant-calling', 'read-alignment', 'gatk', 'filtering', 'annotation', 'vcf-io']) +
      '</div>';
    return win('skills', {}, main);
  };

  SCREENS.scheduler = function () {
    var main = '<div class="bw-scroll">' +
      '<div class="bw-head"><h2>Scheduler</h2>' +
      '<p>Create and manage scheduled tasks to run workflows automatically at specified times.</p>' +
      '<div class="bw-head-actions"><span class="bw-btn dark">' + svg(I.plus) + 'Create Schedule</span>' +
      '<span class="bw-btn soft">' + svg(I.refresh) + 'Refresh</span></div></div>' +
      '<div class="bw-sectionrule"></div>' +
      '<div class="bw-sched"><div class="body">' +
      '<div class="top"><span class="nm">Daily Meditation</span>' +
      '<span class="badge" title="Built into Biorouter.">Built-in</span></div>' +
      '<div class="cron">At 03:00 AM, digests recent sessions into the Soul knowledge base</div>' +
      '<div class="last">Last run: Jun 20, 2026, 3:00 AM</div></div>' +
      '<div class="acts"><span class="a">' + svg(I.edit) + '</span>' +
      '<span class="a">' + svg(I.pause) + '</span>' +
      '<span class="a del">' + svg(I.trash) + '</span></div></div>' +
      '</div>';
    return win('scheduler', {}, main);
  };

  SCREENS.knowledge = function () {
    /* The real "Soul" knowledge base (~/.config/biorouter/knowledge/soul).
       Node colors mirror credColors.ts exactly: hub gold, entity blue,
       concept green, sources by credibility (peer-reviewed deep blue,
       web warm, personal mauve), notes grey. Hubs are larger + labeled. */
    var C = { hub: '#d6b06a', entity: '#6e9cbe', concept: '#83ac88',
      src_peer: '#49659a', src_web: '#d39a73', src_personal: '#c49aac', note: '#a4a4a4' };
    // [x, y, kind, label(optional)] — laid out as a connected graph.
    var N = [
      [388, 210, 'hub', 'Soul'],            // 0
      [250, 250, 'hub', 'Biorouter'],       // 1
      [510, 150, 'hub', 'OMOP CDM'],        // 2
      [300, 120, 'hub', 'EHR'],             // 3
      [560, 300, 'hub', 'R Shiny'],         // 4
      [180, 170, 'entity', 'CRISPR'],       // 5
      [430, 95, 'entity', 'UCSF OMOP'],     // 6
      [600, 200, 'entity', 'RxNorm'],       // 7
      [640, 120, 'entity', 'SNOMED CT'],    // 8
      [150, 280, 'entity'],                 // 9 EHR
      [210, 370, 'entity', 'plotly'],       // 10
      [470, 360, 'entity'],                 // 11 R
      [360, 320, 'concept', 'Clinical Informatics'], // 12
      [120, 360, 'concept', 'HPC'],         // 13
      [620, 380, 'concept', 'SLURM'],       // 14
      [250, 70, 'concept'],                 // 15 Germline editing
      [690, 280, 'concept', 'Chat Recall'], // 16
      [540, 410, 'src_personal', 'Statin / T2D OMOP'], // 17
      [330, 410, 'src_personal'],           // 18 mtcars Shiny
      [110, 120, 'src_web'],                // 19
      [700, 180, 'src_peer'],               // 20
      [400, 150, 'note'],                   // 21
      [470, 250, 'entity'],                 // 22 soul-writer
      [200, 110, 'note'],                   // 23
    ];
    var E = [[0,1],[0,2],[0,3],[0,4],[0,12],[0,22],[0,21],[0,23],[1,5],[1,9],[1,12],[1,13],[1,18],
      [2,6],[2,7],[2,8],[2,17],[2,22],[3,6],[3,9],[4,10],[4,11],[4,18],[4,14],
      [7,8],[7,20],[12,16],[16,7],[5,19],[5,15],[13,14],[9,17],[11,17],[6,21],[3,5]];
    var hubR = 9, r = 5;
    var ne = E.map(function (e) {
      var a = N[e[0]], b = N[e[1]];
      return '<line x1="' + a[0] + '" y1="' + a[1] + '" x2="' + b[0] + '" y2="' + b[1] +
        '" stroke="rgba(119,128,145,0.42)" stroke-width="0.85"/>';
    }).join('');
    var nn = N.map(function (n, i) {
      var isHub = n[2] === 'hub';
      var rad = isHub ? hubR : r;
      var fill = C[n[2]] || '#9a9a9a';
      var label = (isHub || n[3]) ? n[3] : null;
      var lbl = label ? '<text x="' + (n[0] + rad + 3) + '" y="' + (n[1] + 3.5) +
        '" font-size="' + (isHub ? 12 : 11) + '" font-weight="' + (isHub ? 600 : 400) +
        '" fill="#1f242c" font-family="-apple-system, Arial">' + label + '</text>' : '';
      return '<g class="bw-node" style="animation-delay:' + (i % 6) * 0.55 + 's">' +
        '<circle cx="' + n[0] + '" cy="' + n[1] + '" r="' + rad + '" fill="' + fill +
        '" stroke="rgba(31,36,44,0.5)" stroke-width="' + (isHub ? 1.05 : 0.85) + '"/></g>' + lbl;
    }).join('');
    var graph = '<svg class="net" viewBox="0 0 800 470" preserveAspectRatio="xMidYMid meet">' +
      ne + nn + '</svg>';
    var legend = '<div class="bw-legend">' +
      '<span><i style="background:' + C.entity + '"></i>Entity</span>' +
      '<span><i style="background:' + C.concept + '"></i>Concept</span>' +
      '<span><i style="background:' + C.hub + '"></i>Hub</span>' +
      '<span><i style="background:' + C.src_peer + '"></i>Peer reviewed</span>' +
      '<span><i style="background:' + C.src_web + '"></i>Web source</span>' +
      '<span><i style="background:' + C.src_personal + '"></i>Personal source</span></div>';
    var ingest = '<div class="bw-ingest"><div class="ttl">Ingest sources</div>' +
      '<div class="sub">Drop files, paste text, or add a URL.</div>' +
      '<div class="bw-drop"><div class="di">' + svg(I.upload) + '</div>' +
      '<div class="l1">Drag and drop to stage</div>' +
      '<div class="l2">PDF, DOCX, HTML, CSV, Markdown, or plain text</div></div>' +
      '<div class="bw-paste">' + svg(I.clipboard) + 'Paste text</div>' +
      '<div class="nothing">Nothing staged yet</div>' +
      '<div class="bw-digest">Digest Staged Sources</div></div>';
    var kbhead = '<div class="bw-kbhead">' +
      '<span class="nm">Soul</span><span class="cnt">· 24 pages · 34 links</span>' +
      '<span class="grow"></span>' +
      '<span class="ic">' + svg(I.refresh) + '</span>' +
      '<span class="bw-btn soft">' + svg(I.download) + 'Export as .brkb</span>' +
      '<span class="bw-btn soft">' + svg(I.history) + 'Change log</span>' +
      '<span class="bw-btn soft">' + svg(I.folderopen) + 'Open folder</span></div>';
    var gpanel = '<div class="bw-gpanel">' + kbhead +
      '<div class="bw-gcanvas">' + graph + legend + '</div></div>';
    var main = '<div class="bw-scroll" style="padding:22px 30px 14px;flex:0 0 auto">' +
      '<div class="bw-head"><h2>Knowledge</h2><p>Personal, LLM-maintained knowledge bases.</p></div></div>' +
      '<div class="bw-know">' + ingest + gpanel + '</div>';
    return win('knowledge', {}, main);
  };

  SCREENS.models = function () {
    function prow(letter, name, desc, ok) {
      return '<div class="bw-prow bw-stagger"><span class="av">' + letter + '</span>' +
        '<div class="body"><h4>' + name + '</h4><p>' + desc + '</p></div>' +
        (ok ? '<span class="ok">' + svg(I.check) + 'Configured</span>' : '') + '</div>';
    }
    var body = '<div class="bw-prov">' +
      '<div class="back">' + svg(I.back) + 'Back</div>' +
      '<h2>Provider Configuration</h2>' +
      '<div class="lead">Configure your AI model providers. API keys are encrypted and stored locally.</div>' +
      '<div class="bw-grp"><i class="local"></i>Local models</div>' +
      prow('L', 'Llama Server', 'Bundled llama.cpp runtime. Gemma 4 E4B laptop default; Gemma 4 12B on 64 GiB systems.', true) +
      prow('O', 'Ollama', 'Local open source models. Default qwen3.', true) +
      '<div class="bw-grp"><i class="inst"></i>Institutional models</div>' +
      prow('V', 'Versa API Azure', 'UCSF ChatGPT via Azure OpenAI. Default gpt-5.5-2026-04-24.', true) +
      prow('V', 'Versa API Bedrock', 'UCSF Anthropic models via Amazon Bedrock. Default us.anthropic.claude-opus-4-8.', true) +
      '<div class="bw-grp"><i class="comm"></i>Commercial models</div>' +
      prow('A', 'Anthropic', 'Claude and other models from Anthropic.', true) +
      prow('O', 'OpenAI', 'OpenAI models with gpt-6-sol as the default, plus compatible endpoints.', true) +
      prow('G', 'Google Gemini', 'Gemini models from Google AI.', false) +
      prow('R', 'OpenRouter', 'Route to Claude, Gemini, Grok, DeepSeek, Qwen, and more.', false) +
      '</div>';
    var main = '<div class="bw-main" style="background:var(--bg)"><div class="bw-scroll" style="padding:0;overflow:hidden">' + body + '</div></div>';
    return '<div class="bw bw--nosidebar">' + bar({ icons: false }) + '<div class="bw-body">' + main + '</div></div>';
  };

  /* ─── Uniform scaling ──────────────────────────────────────────
     Lay each mockup out at its fixed design width, then scale the whole
     window to exactly fill its column. Layout never reflows, so nothing
     shifts or truncates; the mount's height tracks the scaled height so
     the page flows normally. Recomputed on resize / zoom. */
  function fitOne(mount) {
    var bw = mount.querySelector('.bw');
    if (!bw) return;
    var designW = bw.offsetWidth || 1180;
    var avail = mount.clientWidth;
    if (!avail) return;
    var s = avail / designW;
    bw.style.transform = 'scale(' + s + ')';
    mount.style.height = Math.round(bw.offsetHeight * s) + 'px';
  }
  function fitAll() {
    document.querySelectorAll('[data-screen]').forEach(fitOne);
  }

  /* ─── Mount ────────────────────────────────────────────────── */
  function mountAll() {
    var mounts = document.querySelectorAll('[data-screen]');
    mounts.forEach(function (el) {
      var key = el.getAttribute('data-screen');
      var build = SCREENS[key];
      if (build) {
        try { el.innerHTML = build(); }
        catch (err) { console.error('mockup render failed:', key, err); }
      }
    });
    fitAll();
    // Re-fit after web fonts settle (metrics can change the window height).
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(fitAll);
    // Observe each mount's column width so resize / zoom keep it pixel-perfect.
    if ('ResizeObserver' in window) {
      var ro = new ResizeObserver(function (entries) {
        entries.forEach(function (e) { fitOne(e.target); });
      });
      mounts.forEach(function (el) { ro.observe(el); });
    } else {
      window.addEventListener('resize', fitAll);
    }
    window.addEventListener('resize', fitAll);
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', mountAll);
  } else {
    mountAll();
  }
})();
