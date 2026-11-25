/* ───────────────────────────────────────────────────────────────
   UCSF Biorouter: App UI mockup renderer
   Builds hand-coded, vector recreations of the Biorouter desktop app
   screens and mounts them into [data-screen] containers, replacing the
   old low-resolution screenshots. Pure DOM, so it zooms losslessly and
   reads at the website's own text size.
   Mirrors the real app: sidebar order from AppSidebar.tsx, screen
   layouts from Home, Chat, Chat history, Workflows, Extensions,
   Settings > Chat > Capabilities, Skills, Knowledge, Scheduler and
   Provider configuration. Every page band follows Layout/PageHeader.tsx:
   a title with its help in an InfoTip (the (i) glyph, its text in
   `title`), then the band's actions. No page prints a paragraph under
   its title.
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
    dictionary: '<path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H19a1 1 0 0 1 1 1v18a1 1 0 0 1-1 1H6.5a1 1 0 0 1 0-5H20"/><path d="m8 13 4-7 4 7"/><path d="M9.1 11h5.7"/>',
    knowledge: '<circle cx="12" cy="12" r="3"/><circle cx="5" cy="6" r="1.6"/><circle cx="19" cy="6" r="1.6"/><circle cx="6" cy="18" r="1.6"/><circle cx="18" cy="18" r="1.6"/><path d="M10 10.5L6 7M14 10.5l4-3.5M10 14l-4 3M14 14l4 3"/>',
    apps: '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="M2 8h20"/><path d="M6 4v4"/><path d="M10 4v4"/>',
    settings: '<path d="M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915"/><circle cx="12" cy="12" r="3"/>',
    users: '<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>',
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
    arrowUp: '<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>',
    info: '<circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/>',
    lock: '<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>',
    more: '<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>',
    clipboardList: '<rect width="8" height="4" x="8" y="2" rx="1" ry="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><path d="M12 11h4"/><path d="M12 16h4"/><path d="M8 11h.01"/><path d="M8 16h.01"/>',
    /* The app's hand-drawn Agent glyph (icons/app-icons.tsx), History's "Show subagent runs". */
    agent: '<rect x="4.5" y="8.5" width="15" height="11" rx="3.5"/><path d="M12 8.5V5.75"/><circle cx="12" cy="4.25" r="1.25"/><path d="M9.5 13v1.5M14.5 13v1.5"/>',
    /* The context ring: the arc is what is left of the window. */
    ring: '<circle cx="12" cy="12" r="8" opacity="0.3"/><path d="M12 4a8 8 0 1 1-7.6 5.5"/>',
    /* Filled custom icons (Attach paperclip, Send paper-plane), rendered solid */
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
     primaryItems (Home, New chat), then the seven componentItems that sit behind
     the `Components` disclosure, then settingsItem pinned to the footer. The
     mockup shows the seven expanded, which is one click away in the app.

     There is no History row: History is reached from the Recents list, so the
     history screen below passes a key that matches nothing and no row lights
     up, the same as the app. */
  var NAV = [
    { k: 'home', label: 'Home', icon: 'home' },
    { sep: true },
    { k: 'newchat', label: 'New chat', icon: 'plus' },
    { k: 'crew', label: 'Crew', icon: 'users' },
    { sep: true },
    { k: 'workflows', label: 'Workflows', icon: 'workflows' },
    { k: 'scheduler', label: 'Scheduler', icon: 'scheduler' },
    { k: 'extensions', label: 'Extensions', icon: 'extensions' },
    { k: 'skills', label: 'Skills', icon: 'skills' },
    { k: 'knowledge', label: 'Knowledge', icon: 'knowledge' },
    { k: 'dictionary', label: 'Dictionary', icon: 'dictionary' },
    { k: 'apps', label: 'Applications', icon: 'apps' },
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

  /* Models the mockups bind. The chip shows the display name
     (bottom_menu/modelLabel.ts modelDisplayName) and its tooltip the raw id,
     which `title` carries here; a private model adds the padlock. A chat that
     reads private data (the CDW and OMOP chats below) has to run on a private
     model, so those name the two Versa defaults. */
  var MODELS = {
    local: { name: 'Qwen3.8 27B', id: 'qwen3.8:27b', lock: true },            /* Ollama default */
    versaAzure: { name: 'GPT-5.6 Sol', id: 'gpt-5.6-sol-2026-07-09', lock: true },
    versaBedrock: { name: 'Claude Opus 5.5', id: 'us.anthropic.claude-opus-5-5', lock: true },
    openai: { name: 'GPT-6.2 Astra', id: 'gpt-6.2-astra', lock: false },
  };

  /* Composer: one card and one muted footer line, the same on Home, in a chat
     and in a split pane (ChatInput.tsx, docs/desktop-ui/chat-composer.md).
     The card holds ONE row: `+` (composer/ComposerPlusMenu.tsx, "Add files and
     tools": attach, mention, commands and the chat's Extensions, Skills and
     Knowledge lists, whose counts are only its tooltip), the text, then Send.
     A running turn swaps Send for Stop and sweeps the card's border.
     Under the card, bottom_menu/ComposerFooter.tsx: the working folder's
     basename on the left (the full path is in its hover card); on the right
     the model chip, then the context ring. Home shows the folder and the
     model only. The chat's cost joins the ring once it is above $0; these
     mockups leave it out. The placeholder follows composer/copy.ts. */
  function composer(opts) {
    opts = opts || {};
    var model = opts.model || MODELS.openai;
    var send = opts.working
      ? '<span class="bw-stop" title="Stop"><i></i></span>'
      : '<span class="bw-stop" title="Send" style="background:var(--bg-3);color:var(--text-2)">' + svgI('arrowUp') + '</span>';
    var card = '<div class="bw-composer" style="padding:7px 7px 7px 9px;margin-bottom:6px' +
      (opts.working ? ';border-color:var(--accent-bar)' : '') + '">' +
      '<div class="bw-toolbar" style="gap:10px">' +
      '<span class="bw-stop" title="Add files and tools" style="background:transparent;color:var(--text-2)">' + svgI('plus') + '</span>' +
      '<span class="ph" style="margin:0;flex:1">' + (opts.placeholder || 'Ask a follow-up') + '<span class="caret"></span></span>' +
      send + '</div></div>';
    var right = '<span class="chip" title="' + model.id + '">' + model.name +
      (model.lock ? svgI('lock') : '') + svgI('chevD') + '</span>';
    if (!opts.home) right += '<span class="chip" title="Context window">' + svgI('ring') + '</span>';
    var foot = '<div class="bw-toolbar" style="margin:0 16px 12px 24px;color:var(--text-3)">' +
      '<span class="chip" title="/Users/wgu/Desktop">' + svgI('folder') + 'Desktop</span>' +
      '<span class="grow"></span>' + right + '</div>';
    return card + foot;
  }

  /* ─── Page band (Layout/PageHeader.tsx) ───────────────────────
     The title, an optional count, the help as an InfoTip, then the band's
     actions on the right: a filter (ui/filter-input.tsx, placeholder
     "Filter"), ghost round icon actions, and at most one accent button. */
  function ico(key, size) {
    return '<svg viewBox="0 0 24 24" width="' + size + '" height="' + size + '" fill="none" ' +
      'stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" ' +
      'style="flex-shrink:0">' + I[key] + '</svg>';
  }
  function band(title, opts) {
    opts = opts || {};
    var count = opts.count != null
      ? '<span style="font-size:12px;font-weight:500;color:var(--text-3);font-variant-numeric:tabular-nums">' + opts.count + '</span>'
      : '';
    var info = opts.info
      ? '<span title="' + opts.info + '" style="display:inline-flex;color:var(--text-3)">' + ico('info', 14) + '</span>'
      : '';
    var back = opts.back ? '<span title="Back" style="display:inline-flex;color:var(--text-2)">' + ico('back', 16) + '</span>' : '';
    return '<div class="bw-headrow" style="align-items:center">' +
      '<div class="bw-head" style="display:flex;align-items:center;gap:8px;margin:0;min-width:0">' +
      back + '<h2>' + title + '</h2>' + count + info + (opts.after || '') + '</div>' +
      '<div class="bw-head-actions" style="margin-top:0;align-items:center">' + (opts.actions || '') + '</div></div>' +
      (opts.noRule ? '' : '<div class="bw-sectionrule" style="margin-top:14px"></div>');
  }
  function filterBox(placeholder) {
    return '<span style="display:inline-flex;align-items:center;gap:6px;width:150px;height:28px;padding:0 9px;' +
      'border-radius:8px;background:var(--bg-3);color:var(--text-3);font-size:12px">' +
      ico('search', 13) + (placeholder || 'Filter') + '</span>';
  }
  function iconBtn(key, label) {
    return '<span title="' + label + '" style="width:28px;height:28px;border-radius:50%;display:inline-flex;' +
      'align-items:center;justify-content:center;color:var(--text-2)">' + ico(key, 15) + '</span>';
  }
  function primaryBtn(label) {
    return '<span class="bw-btn accent">' + svg(I.plus) + label + '</span>';
  }
  /* A caps group label carrying its count (Crew's form, no dot). */
  function groupLabel(label, count) {
    return '<div class="bw-grp" style="margin:16px 0 2px">' + label +
      ' <span style="font-variant-numeric:tabular-nums">' + count + '</span></div>';
  }
  /* ui/PrivacyBadge.tsx: Private is a padlock AND the word; Public says its word. */
  function privacyBadge(tier) {
    var priv = tier === 'private';
    return '<span style="display:inline-flex;align-items:center;gap:3px;font-size:10px;font-weight:600;' +
      'padding:1px 7px;border-radius:5px;background:var(--bg-3);color:' + (priv ? 'var(--text)' : 'var(--text-2)') + '">' +
      (priv ? ico('lock', 10) : '') + (priv ? 'Private' : 'Public') + '</span>';
  }
  function plainBadge(text) {
    return '<span style="font-size:10px;font-weight:600;padding:1px 7px;border-radius:5px;' +
      'background:var(--bg-3);color:var(--text-2)">' + text + '</span>';
  }

  /* ─── Screen builders ──────────────────────────────────────── */
  var SCREENS = {};

  SCREENS.home = function () {
    /* Home (sessions/SessionsInsights.tsx): the greeting, one sentence drawn
       from the stock set in common/Greeting.tsx on each arrival, then ONE usage
       card (sessions/HomeUsageCard.tsx). No eyebrow and no recent chats: the
       sidebar owns those. The card: Overview | Models tabs and the 1y, 30d,
       7d range; six tiles (Chats, Messages, Tokens, Active days, Streak,
       Favorite model); the activity heatmap, always the whole year; and one
       footnote line, the streak and the Less ... More legend. */
    function tile(label, value, suffix) {
      return '<div class="bw-stat"><div class="lbl" style="margin-bottom:4px">' + label + '</div>' +
        '<div class="n" style="font-size:19px;font-weight:400">' + value +
        (suffix ? '<span style="font-size:12px;color:var(--text-3)">' + suffix + '</span>' : '') +
        '</div></div>';
    }
    var shades = ['var(--bg-4)', 'rgba(207,109,71,0.28)', 'rgba(207,109,71,0.5)',
      'rgba(207,109,71,0.75)', 'var(--accent-bar)'];
    var cells = '';
    for (var i = 0; i < 52 * 7; i++) {
      var v = (i * 37 + (i % 7) * 11) % 13;
      var lvl = v < 5 ? 0 : v < 8 ? 1 : v < 10 ? 2 : v < 12 ? 3 : 4;
      cells += '<i style="display:block;border-radius:2px;background:' + shades[lvl] + '"></i>';
    }
    var heatmap = '<div style="display:grid;grid-template-rows:repeat(7,8px);grid-auto-flow:column;' +
      'grid-auto-columns:8px;gap:2px;margin:4px 0 10px">' + cells + '</div>';
    var legend = '<span style="display:inline-flex;align-items:center;gap:3px">Less' +
      shades.map(function (s) {
        return '<i style="display:inline-block;width:8px;height:8px;border-radius:2px;background:' + s + '"></i>';
      }).join('') + 'More</span>';
    var head = '<div style="display:flex;align-items:center;justify-content:space-between;height:36px;' +
      'border-bottom:1px solid var(--border);font-size:12px">' +
      '<span style="display:inline-flex;gap:16px;align-self:stretch">' +
      '<span style="display:inline-flex;align-items:center;font-weight:600;color:var(--text);box-shadow:inset 0 -2px 0 var(--accent-bar)">Overview</span>' +
      '<span style="display:inline-flex;align-items:center;color:var(--text-2)">Models</span></span>' +
      '<span style="display:inline-flex;gap:2px;padding:2px;border-radius:7px;background:var(--bg-4);font-size:11px">' +
      '<span style="padding:1px 8px;border-radius:5px;background:var(--bg-2);color:var(--text)">1y</span>' +
      '<span style="padding:1px 8px;color:var(--text-2)">30d</span>' +
      '<span style="padding:1px 8px;color:var(--text-2)">7d</span></span></div>';
    var card = '<div style="max-width:620px;background:var(--bg-3);border-radius:12px;padding:0 16px 10px">' + head +
      '<div style="display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:12px 18px;padding:14px 0 12px">' +
      tile('Chats', '1,122') + tile('Messages', '18,406') + tile('Tokens', '239.2M') +
      tile('Active days', '214', '/365') + tile('Streak', '12', ' days') + tile('Favorite model', 'GPT-5.6 Sol') +
      '</div>' + heatmap +
      '<div style="display:flex;justify-content:space-between;font-size:10.5px;color:var(--text-3)">' +
      '<span>12 day streak · longest 31 days</span>' + legend + '</div></div>';
    var main = '<div class="bw-scroll bw-home">' +
      '<h1>Which medical mystery might the knowledge graph help solve today?</h1>' + card +
      '</div>' + composer({ home: true, placeholder: 'Ask Biorouter anything…', model: MODELS.local });
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
    /* A running turn shows as the composer's Stop and its border sweep; the
       transcript prints no "working" line. */
    var main = '<div class="bw-scroll" style="overflow:hidden">' +
      '<div class="bw-chat-title">New chat</div>' +
      '<div class="bw-bubble">How many patients with type 2 diabetes are currently receiving statin therapy?</div>' +
      '<div class="bw-bubble-time">4:52 PM</div>' + rows +
      '</div>' + composer({ working: true, model: MODELS.versaAzure });
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
      '</div>' + composer({ working: true, model: MODELS.versaBedrock });
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
      '</div>' + composer({ working: true, model: MODELS.openai });
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
    /* The band (sessions/SessionListView.tsx, sessions/copy.ts): the title
       with its help in an InfoTip, the filter whose placeholder carries the
       shortcut, then two icon actions, Show subagent runs and Import chat. */
    var main = '<div class="bw-scroll">' +
      band('Chat history', {
        info: 'Every chat you have had with Biorouter. Open one to pick it up where you left off.',
        actions: filterBox('Search history… ⌘F') + iconBtn('agent', 'Show subagent runs') +
          iconBtn('upload', 'Import chat'),
      }) +
      '<div class="bw-day">Today</div>' +
      hrow('New chat', '4:52 PM', '1', '16,503', '10') +
      hrow('Remove codegraph indices folder', '1:14 PM', '87', '41,834', '9') +
      hrow('New chat', '1:12 PM', '12', '3,356', '2') +
      hrow('GWAS on Type 2 Diabetes', '1:07 PM', '27', '102,443', '10') +
      hrow('EHR of Patients with Multiple Sclerosis', '1:07 PM', '4', '25,478', '10') +
      hrow('Functional Significance of the EPAS1 Gene', '1:06 PM', '20', '49,765', '10') +
      '<div class="bw-day">Monday, June 1</div>' +
      hrow('MS genes SPOKE query', '7:21 PM', '18', '29,652', '10') +
      '</div>';
    return win('history', {}, main);
  };

  SCREENS.workflows = function () {
    /* workflows/WorkflowsView.tsx: the band holds the filter, Import as a
       ghost round icon and "New workflow" as the view's one accent. A row is
       the title (with its slash command, schedule or Built-in badges) and one
       muted line of description. At rest the trailing edge shows when it was
       modified; hover or focus gives way to Run and the ⋯ menu, which holds
       every other verb. */
    function wrow(title, desc, trailing) {
      return '<div class="bw-wf bw-stagger" style="align-items:center;padding:12px 4px"><div class="body">' +
        '<h4 style="margin-bottom:2px">' + title + '</h4>' +
        '<p style="white-space:nowrap;overflow:hidden;text-overflow:ellipsis">' + desc + '</p></div>' +
        trailing + '</div>';
    }
    var hovered = '<div class="bw-actions"><span class="a" title="Run">' + svgI('play') + '</span>' +
      '<span class="a" title="More actions">' + svgI('more') + '</span></div>';
    var atRest = '<span style="font-size:11px;color:var(--text-3);font-variant-numeric:tabular-nums">Nov 3</span>';
    var main = '<div class="bw-scroll">' +
      band('Workflows', {
        info: 'Reusable chat setups. Start one from here or with its slash command.',
        actions: filterBox() + iconBtn('upload', 'Import workflow') + primaryBtn('New workflow'),
      }) +
      wrow('Scientific Article Writing and Reference Validator',
        'A workflow for converting informal or plain-language content into formally structured scientific articles, and validating all cited references against live academic databases.',
        hovered) +
      wrow('EHR Diabetes Demographics and Reporting Dashboard',
        'A workflow for querying an OMOP-based EHR to derive a disease cohort (e.g., T2D), summarize demographics, and produce sharable visual outputs.',
        atRest) +
      '</div>';
    return win('workflows', {}, main);
  };

  /* One installed extension (settings/extensions/subcomponents/ExtensionItem.tsx):
     a glyph, the name with its Privacy badge, ONE muted line (the description,
     then the provenance sentence from baam/registry.ts), the ⋯ menu on hover,
     and the switch. */
  function erow(name, tier, desc, provenance, on) {
    return '<div class="bw-erow bw-stagger" style="padding:11px 4px">' +
      '<span style="display:inline-flex;color:var(--text-2)">' + ico('extensions', 16) + '</span>' +
      '<div class="body"><h4 style="display:flex;align-items:center;gap:6px">' + name + privacyBadge(tier) + '</h4>' +
      '<p style="white-space:nowrap;overflow:hidden;text-overflow:ellipsis">' + desc + ' · ' + provenance + '</p></div>' +
      '<span class="bw-switch' + (on ? ' on' : '') + '"></span></div>';
  }

  SCREENS.extensions = function () {
    /* extensions/ExtensionsView.tsx: the band holds the title, its count,
       the help, the filter and ONE Add menu (Browse marketplace, Install from
       file (.brxt), Custom extension…). The list groups what is installed
       into Enabled and Available. Biorouter's own capabilities (Developer,
       Biorouter Copilot and the rest) have no row here: they live under
       Settings > Chat > Capabilities (SCREENS.capabilities). */
    var listed = 'Public: published on the Biorouter marketplace';
    var main = '<div class="bw-scroll">' +
      band('Extensions', {
        count: 4,
        info: 'Extensions add tools, prompts and resources. Enabled ones apply to every new chat.',
        actions: filterBox() + primaryBtn('Add'),
      }) +
      groupLabel('Enabled', 3) +
      erow('CDWAgent', 'private', 'Multimodal access to the UCSF Clinical Data Warehouse through natural language',
        'Private: published on the Biorouter marketplace', true) +
      erow('SPOKEAgent', 'public', 'Structure-aware access to the SPOKE biomedical knowledge graph', listed, true) +
      erow('Playwright Agent', 'public', 'Browser automation via Microsoft’s @playwright/mcp', listed, true) +
      groupLabel('Available', 1) +
      erow('CodeGraph Agent', 'public', 'Pre-indexed code knowledge graph', listed, false) +
      '</div>';
    return win('extensions', {}, main);
  };

  SCREENS.capabilities = function () {
    /* Settings > Chat > Capabilities (settings/capabilities/CapabilitiesSection.tsx):
       the built-in abilities new chats start with, which ship in
       settings/extensions/bundled-extensions.json. Each row is the name, its
       description as an InfoTip, and the switch. A chat can still switch one
       for itself from the composer's + popover. */
    function crow(name, desc) {
      return '<div class="bw-erow bw-stagger" style="padding:10px 4px"><div class="body">' +
        '<h4 style="display:flex;align-items:center;gap:6px">' + name +
        '<span title="' + desc + '" style="display:inline-flex;color:var(--text-3)">' + ico('info', 13) + '</span></h4></div>' +
        '<span class="bw-switch on"></span></div>';
    }
    var tabs = '<span style="display:inline-flex;gap:16px;margin-left:18px;font-size:12px;align-self:stretch">' +
      '<span style="color:var(--text-2)">Models</span>' +
      '<span style="font-weight:600;color:var(--text);box-shadow:inset 0 -2px 0 var(--accent-bar)">Chat</span>' +
      '<span style="color:var(--text-2)">App</span></span>';
    var main = '<div class="bw-scroll">' +
      band('Settings', { after: tabs }) +
      '<div class="bw-grp" style="margin:16px 0 2px;display:flex;align-items:center;gap:6px">Capabilities' +
      '<span title="Choose which built-in abilities new chats start with. Existing chats keep their current capabilities." ' +
      'style="display:inline-flex">' + ico('info', 12) + '</span></div>' +
      crow('Developer', 'Read, write and run code, and run shell commands.') +
      crow('Biorouter Copilot', 'View and control desktop apps for an approved task. You allow each task from its chat, and Stop stays above the composer.') +
      crow('Web &amp; Documents', 'Read web pages and work with spreadsheets, documents, and PDFs.') +
      crow('Auto Visualiser', 'Interactive charts, diagrams, networks, maps and scientific plots, shown in the side panel.') +
      crow('Memory', 'Teach Biorouter your preferences so it remembers them as you go.') +
      crow('Knowledge', 'Personal knowledge bases Biorouter builds and maintains for you, stored as markdown with full history.') +
      crow('Application Drafter', 'Build static pages or interactive applications with an embedded Biorouter agent, and export them as standalone projects.') +
      '</div>';
    return win('settings', {}, main);
  };

  SCREENS.skills = function () {
    /* skills/SkillsView.tsx: the band holds the title, its count, the help,
       the filter and ONE Add menu (Browse marketplace, From a repository or
       .zip…, Write a skill…). Groups are by provenance (Biorouter, From
       <extension>, From other agents, From this project). A package is one
       row that opens in place ("13 skills", then its version when it has
       one); a single skill is its name and one muted line of description;
       each row has the ⋯ menu and a switch. The packs below are the BAAM
       marketplace's, with the skill counts registry.json gives them. */
    function srow(name, line, bundle) {
      return '<div class="bw-erow bw-stagger" style="padding:10px 4px"><div class="body">' +
        '<h4 style="display:flex;align-items:center;gap:4px">' + name +
        (bundle ? '<span style="display:inline-flex;color:var(--text-3)">' + ico('chevR', 14) + '</span>' : '') + '</h4>' +
        '<p>' + line + '</p></div>' +
        '<span class="bw-switch on"></span></div>';
    }
    var main = '<div class="bw-scroll">' +
      band('Skills', {
        count: 5,
        info: 'Reusable instructions Biorouter follows when a task calls for them.',
        actions: filterBox() + primaryBtn('Add'),
      }) +
      groupLabel('Biorouter', 5) +
      srow('Superpowers Collection', '13 skills', true) +
      srow('Clinical Databases', '10 skills', true) +
      srow('Single-cell', '14 skills', true) +
      srow('Variant Calling', '13 skills', true) +
      srow('ggplot-visualization', 'Applies ggplot2 best-practice style when writing R plotting code.') +
      '</div>';
    return win('skills', {}, main);
  };

  SCREENS.scheduler = function () {
    /* schedule/SchedulesView.tsx: the band holds Refresh schedules as a ghost
       round icon and "New schedule". A row is the status dot, the name and its
       Built-in badge, then one line: the state, the rule in words
       (schedule/cronPhrase.ts) and the last run. Pause, Resume or Stop sits
       beside the ⋯ menu (Edit, Run now, Delete…). The built-in Daily
       Meditation runs at 3:00 AM and digests recent chats into Soul. */
    var main = '<div class="bw-scroll">' +
      band('Scheduler', {
        info: 'Runs a saved workflow on a timer. Each run opens a new chat.',
        actions: iconBtn('refresh', 'Refresh schedules') + primaryBtn('New schedule'),
      }) +
      '<div class="bw-sched" style="align-items:center">' +
      '<span style="width:7px;height:7px;border-radius:50%;background:#4a9d6b;flex-shrink:0"></span>' +
      '<div class="body">' +
      '<div class="top"><span class="nm">Daily Meditation</span>' +
      '<span class="badge" title="Built into Biorouter.">Built-in</span></div>' +
      '<div class="cron">Scheduled · Every day at 3:00 AM · Last run Nov 14, 3:00 AM</div></div>' +
      '<div class="acts"><span class="a" title="Pause Daily Meditation" style="border-color:transparent">' + svg(I.pause) + '</span>' +
      '<span class="a" title="More actions" style="border-color:transparent">' + svg(I.more) + '</span></div></div>' +
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
    // [x, y, kind, label(optional)], laid out as a connected graph.
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
    /* The Sources rail (knowledge/IngestPanel): the drop zone, Paste text,
       the staged list once something is staged (nothing is here, so it draws
       nothing), and at the foot the digest model and "Digest staged sources". */
    var ingest = '<div class="bw-ingest">' +
      '<div class="bw-drop"><div class="di">' + svg(I.upload) + '</div>' +
      '<div class="l1">Drag and drop to stage</div>' +
      '<div class="l2">PDF, Office, web pages, folders</div></div>' +
      '<div class="bw-paste">' + svg(I.clipboard) + 'Paste text</div>' +
      '<div style="margin-top:auto;display:flex;flex-direction:column;gap:8px">' +
      '<div title="' + MODELS.local.id + '" style="display:flex;align-items:center;justify-content:space-between;' +
      'padding:7px 10px;border-radius:8px;background:var(--bg-3);font-size:12px;color:var(--text)">' +
      MODELS.local.name + '<span style="display:inline-flex;color:var(--text-3)">' + ico('chevD', 13) + '</span></div>' +
      '<div class="bw-digest" style="margin-top:0;background:var(--accent)">Digest staged sources</div></div></div>';
    /* The ⋯ menu, drawn open: Check for problems and Refresh graph sit in the
       band, and everything else about the base is here. */
    var menu = '<div style="position:absolute;top:8px;right:14px;z-index:2;min-width:190px;padding:4px;' +
      'border:1px solid var(--border);border-radius:10px;background:var(--bg-2);box-shadow:var(--shadow-float);font-size:12px">' +
      ['Export as .brkb…', 'Change log', 'Open folder'].map(function (t) {
        return '<div style="padding:6px 9px;border-radius:6px;color:var(--text)">' + t + '</div>';
      }).join('') +
      '<div style="height:1px;margin:4px 6px;background:var(--border)"></div>' +
      '<div style="padding:6px 9px;border-radius:6px;color:#c0563a">Delete knowledge base…</div></div>';
    var gpanel = '<div class="bw-gpanel">' +
      '<div class="bw-gcanvas">' + menu + graph + legend + '</div></div>';
    /* One band (knowledge/KnowledgeView.tsx): "Knowledge" with its help in an
       InfoTip, "/", the base's name (which IS the switcher), the counts, then
       Check for problems, Refresh graph and the ⋯ menu. */
    var subject = '<span style="color:var(--text-3);font-size:14px">/</span>' +
      '<span style="display:inline-flex;align-items:center;gap:3px;font-size:13px;font-weight:600;color:var(--text)">Soul' +
      '<span style="display:inline-flex;color:var(--text-3)">' + ico('chevD', 13) + '</span></span>' +
      '<span style="font-size:11px;color:var(--text-3)">24 pages · 34 links</span>';
    var main = '<div class="bw-scroll" style="padding:18px 30px 14px;flex:0 0 auto">' +
      band('Knowledge', {
        info: 'Personal knowledge bases Biorouter builds and maintains for you.',
        after: subject,
        noRule: true,
        actions: iconBtn('clipboardList', 'Check for problems') + iconBtn('refresh', 'Refresh graph') +
          '<span title="More knowledge base actions" style="width:28px;height:28px;border-radius:50%;display:inline-flex;' +
          'align-items:center;justify-content:center;color:var(--text);background:var(--bg-3)">' + ico('more', 15) + '</span>',
      }) + '</div>' +
      '<div class="bw-know">' + ingest + gpanel + '</div>';
    return win('knowledge', {}, main);
  };

  SCREENS.models = function () {
    /* Provider configuration (settings/providers/ProviderSettingsPage.tsx and
       ProviderCatalog.tsx): one band with Back, the title and three tabs on
       the title's line, always Local · Institutional · Public, each with its
       tier dot. One tab shows at a time, under its tier note
       (providerOrdering.ts). The Local tab is drawn here: Llama Server first,
       its catalog default Qwen3.8 27B (Gemma 4 31B also offered), then Ollama.
       Institutional holds the gateways Biorouter recognises (Versa API Azure,
       Versa API Bedrock); Public holds "AI agents · your subscription" (Claude
       Code and Codex, which run on the user's own signed-in CLI plan, no API
       key) and then the API providers (Anthropic, OpenAI, Google Gemini,
       OpenRouter, Baseten and the rest). */
    function prow(letter, name, desc, model, ok) {
      return '<div class="bw-prow bw-stagger"><span class="av">' + letter + '</span>' +
        '<div class="body"><h4>' + name + '</h4><p>' + desc + '</p>' +
        (model ? '<p style="color:var(--text-3)">' + model + '</p>' : '') + '</div>' +
        (ok ? '<span style="font-size:11px;color:var(--text-3)">Configured</span>' : '') +
        '<span class="bw-btn soft" style="padding:5px 11px">Configure</span></div>';
    }
    function tab(label, dot, on) {
      return '<span style="display:inline-flex;align-items:center;gap:6px;' +
        (on ? 'font-weight:600;color:var(--text);box-shadow:inset 0 -2px 0 var(--accent-bar)' : 'color:var(--text-2)') + '">' +
        '<i style="width:6px;height:6px;border-radius:50%;background:' + dot + '"></i>' + label + '</span>';
    }
    var tabs = '<span style="display:inline-flex;gap:18px;margin-left:18px;font-size:12px;align-self:stretch">' +
      tab('Local', '#4a9d6b', true) + tab('Institutional', 'var(--accent)', false) + tab('Public', '#e0a23a', false) +
      '</span>';
    var body = '<div class="bw-prov" style="max-width:680px">' +
      band('Provider configuration', { back: true, after: tabs, noRule: true }) +
      '<div class="lead" style="margin-top:12px">Private because inference runs on this machine. Nothing leaves it.</div>' +
      prow('L', 'Llama Server', 'Built-in local models (llama.cpp): private, free, no setup required',
        'Qwen3.8 27B · 19 GB by default, Gemma 4 31B also offered', true) +
      prow('O', 'Ollama', 'Local open source models', 'qwen3.8:27b by default', true) +
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
