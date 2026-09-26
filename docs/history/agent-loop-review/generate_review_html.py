#!/usr/bin/env python3
"""Assemble the BioRouter Agentic Loop Review markdown corpus into one
self-contained, design-system-faithful HTML report. pandoc does md->html;
this script themes and stitches. Output lands in the task worktree only."""
import subprocess, re, html, sys, pathlib

# Resolve the review corpus relative to this script so it works from any checkout.
SRC = pathlib.Path(__file__).resolve().parent
OUT = SRC / "review.html"  # generated view; not checked in (the corpus is the source of truth)

# (group, key, relpath, friendly title)
DOCS = [
    ("Overview", "readme",            "README.md",              "Executive Report"),
    ("Overview", "proposals-master",  "improvement-proposals.md",           "The 67-Proposal Improvement Program"),

    ("Internal reviews", "internal-core-loop",             "subsystem-reviews/core-loop-and-tool-dispatch.md",             "Core agent loop & tool dispatch"),
    ("Internal reviews", "internal-context-injection",     "subsystem-reviews/context-injection-and-system-prompt.md",     "Context injection & system prompt"),
    ("Internal reviews", "internal-compaction",            "subsystem-reviews/compaction-and-context-management.md",            "Compaction & context management"),
    ("Internal reviews", "internal-hooks",                 "subsystem-reviews/hooks-system.md",                 "Hooks system"),
    ("Internal reviews", "internal-guardrails-permissions","subsystem-reviews/guardrails-and-permissions.md","Guardrails, security & permissions"),
    ("Internal reviews", "internal-loop-detection",        "subsystem-reviews/loop-and-stuck-detection.md",        "Loop detection & stuck states"),
    ("Internal reviews", "internal-long-running",          "subsystem-reviews/long-running-tasks-and-scheduling.md",          "Long-running tasks & processes"),
    ("Internal reviews", "internal-state-awareness",       "subsystem-reviews/state-awareness-and-version-control.md",       "State, todos, goals & version control"),
    ("Internal reviews", "internal-server-flow",           "subsystem-reviews/server-reply-flow-and-session-lifecycle.md",           "Server reply flow & session lifecycle"),
    ("Internal reviews", "internal-verification",          "subsystem-reviews/self-verification-and-doneness.md",          "Self-verification & quality checkpoints"),

    ("External studies", "external-goose",       "../../research/coding-agent-landscape/goose.md",       "Goose (Block)"),
    ("External studies", "external-cline",       "../../research/coding-agent-landscape/cline.md",       "Cline"),
    ("External studies", "external-opencode",    "../../research/coding-agent-landscape/opencode.md",    "OpenCode (sst)"),
    ("External studies", "external-pi",          "../../research/coding-agent-landscape/pi.md",          "Pi (badlogic)"),
    ("External studies", "external-aider",       "../../research/coding-agent-landscape/aider.md",       "Aider"),
    ("External studies", "external-openhands",   "../../research/coding-agent-landscape/openhands.md",   "OpenHands"),
    ("External studies", "external-codex-cli",   "../../research/coding-agent-landscape/codex-cli.md",   "OpenAI Codex CLI"),
    ("External studies", "external-gemini-cli",  "../../research/coding-agent-landscape/gemini-cli.md",  "Gemini CLI"),
    ("External studies", "external-claude-code", "../../research/coding-agent-landscape/claude-code.md", "Claude Code"),

    ("Comparisons", "compare-context",   "competitive-comparison/context-and-prompts.md",   "Context & environment awareness"),
    ("Comparisons", "compare-memory",    "competitive-comparison/compaction-and-memory.md",    "Compaction, memory & continuity"),
    ("Comparisons", "compare-safety",    "competitive-comparison/safety-and-guardrails.md",    "Hooks, permissions, guardrails & loops"),
    ("Comparisons", "compare-execution", "competitive-comparison/execution-and-verification.md", "Tool loop, tasks, checkpoints & verification"),

    ("Proposal lenses", "proposals-performance", "proposal-lenses/performance.md", "Performance & efficiency lens"),
    ("Proposal lenses", "proposals-robustness",  "proposal-lenses/robustness.md",  "Robustness & safety lens"),
    ("Proposal lenses", "proposals-ux",          "proposal-lenses/ux.md",          "Usability & agent-ergonomics lens"),
]

# map source relpath -> in-page anchor, for cross-reference rewriting
PATH_TO_ANCHOR = {}
for _g, key, rel, _t in DOCS:
    PATH_TO_ANCHOR[rel] = "doc-" + key
    PATH_TO_ANCHOR["./" + rel] = "doc-" + key

def pandoc(md_path, idprefix):
    res = subprocess.run(
        ["pandoc", "--from", "gfm", "--to", "html5", "--no-highlight",
         "--wrap=none", f"--id-prefix={idprefix}-", str(md_path)],
        capture_output=True, text=True)
    if res.returncode != 0:
        sys.stderr.write(f"pandoc failed on {md_path}: {res.stderr}\n")
        sys.exit(1)
    return res.stdout

def rewrite_links(frag):
    # Clickable in-page cross-references: internal/foo.md#x -> #doc-internal-foo
    def repl(m):
        grp, name, anchor = m.group(1), m.group(2), m.group(3) or ""
        target = f"{grp}/{name}.md"
        a = PATH_TO_ANCHOR.get(target) or PATH_TO_ANCHOR.get("./" + target)
        return f'href="#{a}"' if a else m.group(0)
    frag = re.sub(r'href="(?:\.{1,2}/)*(internal|external|compare|proposals)/([a-z0-9-]+)\.md(#[^"]*)?"', repl, frag)
    frag = re.sub(r'href="(?:\.{1,2}/)*PROPOSALS\.md(#[^"]*)?"', 'href="#doc-proposals-master"', frag)
    frag = re.sub(r'href="(?:\.{1,2}/)*README\.md(#[^"]*)?"', 'href="#doc-readme"', frag)
    return frag

def wrap_tables(frag):
    return re.sub(r'(<table[\s\S]*?</table>)', r'<div class="tablewrap">\1</div>', frag)

def strip_leading_h1(frag):
    # each doc gets a friendly header already; drop its own first H1
    return re.sub(r'^\s*<h1[^>]*>[\s\S]*?</h1>', '', frag, count=1)

fragments = []
for group, key, rel, title in DOCS:
    p = SRC / rel
    if not p.exists():
        sys.stderr.write(f"MISSING {p}\n"); sys.exit(1)
    frag = pandoc(p, key)
    frag = strip_leading_h1(frag)
    frag = rewrite_links(frag)
    frag = wrap_tables(frag)
    fragments.append((group, key, rel, title, frag))

# ---- build nav ----
nav_html = []
last_group = None
group_num = 0
for group, key, rel, title, _frag in fragments:
    if group != last_group:
        group_num += 1
        nav_html.append(f'<div class="rail__grp">{html.escape(group)}</div>')
        last_group = group
    nav_html.append(
        f'<a class="rail__link" href="#doc-{key}"><span class="rail__t">{html.escape(title)}</span></a>')
nav_html = "\n".join(nav_html)

# ---- build doc sections ----
sec_html = []
counters = {}
group_index = {}
gi = 0
for group, key, rel, title, frag in fragments:
    if group not in group_index:
        gi += 1; group_index[group] = gi; counters[group] = 0
    counters[group] += 1
    num = f"{group_index[group]:02d}.{counters[group]}"
    fname = html.escape(rel)
    sec_html.append(f'''
<section class="doc" id="doc-{key}">
  <header class="doc__head">
    <div class="doc__meta"><span class="doc__num">{num}</span><span class="doc__tag">{html.escape(group)}</span><span class="doc__file">{fname}</span></div>
    <h2 class="doc__title">{html.escape(title)}</h2>
  </header>
  <div class="doc__body">
{frag}
  </div>
  <a class="doc__top" href="#top" aria-label="Back to top">↑ top</a>
</section>''')
sec_html = "\n".join(sec_html)

CSS = r"""
:root{
  --canvas:#faf8f3; --surface:#ffffff; --fill:#f4f0e6; --fill-strong:#e8e1d2;
  --ink:#2a2520; --muted:#6e6760; --subtle:#8d867d;
  --line:#e8e1d2; --line-soft:#efe9df; --line-strong:#d4cab6;
  --accent:#b85a32; --accent-hover:#a94f2a; --accent-soft:#f6e9e2; --accent-bar:#cf6d47;
  --danger:#b3261e; --success:#1f7a3d; --warning:#8a5a00; --info:#1e5fbf;
  --code-bg:#f5f1e8; --code-ink:#2a2520;
  --c-kw:#a94f2a; --c-str:#22784f; --c-num:#8a5a00; --c-fn:#255fb5; --c-com:#6f6659; --c-type:#7847b8;
  --scrim: rgba(32,25,15,.14);
  --shadow: 0 1px 2px rgba(35,32,28,.04), 0 12px 32px -22px rgba(35,32,28,.22);
  --rail:288px; --measure:1100px;
  --sans: ui-sans-serif,-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,"Helvetica Neue",Arial,sans-serif;
  --mono:"JetBrains Mono",ui-monospace,"SF Mono",SFMono-Regular,"Cascadia Mono",Menlo,Consolas,monospace;
  --display:"Fraunces",Georgia,"Times New Roman",serif;
  color-scheme: light;
}
@media (prefers-color-scheme: dark){ :root:not([data-theme="light"]){
  --canvas:#0d0a06; --surface:#16120c; --fill:#282217; --fill-strong:#3a3324;
  --ink:#f4f0e6; --muted:#b0a892; --subtle:#88806a;
  --line:#282217; --line-soft:#211c15; --line-strong:#403928;
  --accent:#e8895f; --accent-hover:#f0a07c; --accent-soft:#2b1c14; --accent-bar:#e8895f;
  --danger:#f07575; --success:#7ac87c; --warning:#f0c84a; --info:#7aabf5;
  --code-bg:#120e08; --code-ink:#e8e1d2;
  --c-kw:#e8895f; --c-str:#7fbf6a; --c-num:#d9a441; --c-fn:#8fb8e8; --c-com:#8d8266; --c-type:#b98ad6;
  --scrim: rgba(0,0,0,.5);
  --shadow: 0 1px 2px rgba(0,0,0,.4), 0 12px 32px -22px rgba(0,0,0,.7);
  color-scheme: dark;
}}
:root[data-theme="dark"]{
  --canvas:#0d0a06; --surface:#16120c; --fill:#282217; --fill-strong:#3a3324;
  --ink:#f4f0e6; --muted:#b0a892; --subtle:#88806a;
  --line:#282217; --line-soft:#211c15; --line-strong:#403928;
  --accent:#e8895f; --accent-hover:#f0a07c; --accent-soft:#2b1c14; --accent-bar:#e8895f;
  --danger:#f07575; --success:#7ac87c; --warning:#f0c84a; --info:#7aabf5;
  --code-bg:#120e08; --code-ink:#e8e1d2;
  --c-kw:#e8895f; --c-str:#7fbf6a; --c-num:#d9a441; --c-fn:#8fb8e8; --c-com:#8d8266; --c-type:#b98ad6;
  --scrim: rgba(0,0,0,.5);
  color-scheme: dark;
}

*{box-sizing:border-box;}
html{scroll-behavior:smooth;}
body{margin:0;background:var(--canvas);color:var(--ink);
  font:400 15.5px/1.65 var(--sans);-webkit-font-smoothing:antialiased;text-rendering:optimizeLegibility;}
a{color:var(--accent);text-decoration:none;text-underline-offset:2px;}
a:hover{text-decoration:underline;}

.shell{display:grid;grid-template-columns:var(--rail) minmax(0,1fr);}

/* ---- rail ---- */
.rail{position:sticky;top:0;align-self:start;height:100dvh;overflow-y:auto;
  border-right:1px solid var(--line);background:var(--surface);
  padding:26px 16px 40px;display:flex;flex-direction:column;gap:20px;}
.brand{display:flex;align-items:center;gap:11px;padding:0 6px;}
.brand__mark{width:30px;height:30px;flex:none;border-radius:8px;}
.brand__name{font:600 15px/1.1 var(--sans);letter-spacing:-.01em;}
.brand__sub{font:500 10px/1.3 var(--mono);letter-spacing:.1em;text-transform:uppercase;color:var(--subtle);margin-top:2px;}
.rail__nav{display:flex;flex-direction:column;gap:1px;}
.rail__grp{font:600 9.5px/1 var(--mono);letter-spacing:.14em;text-transform:uppercase;
  color:var(--accent);margin:18px 0 6px 8px;}
.rail__link{display:block;padding:6px 9px;border-radius:7px;color:var(--muted);
  font:450 13px/1.35 var(--sans);border-left:2px solid transparent;transition:background-color .12s,color .12s;}
.rail__link:hover{background:var(--fill);color:var(--ink);text-decoration:none;}
.rail__link.is-active{background:var(--accent-soft);color:var(--ink);border-left-color:var(--accent-bar);font-weight:550;}
.rail__foot{margin-top:auto;display:flex;flex-direction:column;gap:8px;padding:0 6px;}
.themetoggle{display:flex;gap:2px;padding:3px;border:1px solid var(--line);border-radius:9px;background:var(--canvas);}
.themetoggle button{flex:1;border:0;background:transparent;color:var(--muted);
  font:500 11.5px var(--sans);padding:5px 6px;border-radius:6px;cursor:pointer;}
.themetoggle button[aria-pressed="true"]{background:var(--surface);color:var(--ink);box-shadow:var(--shadow);}
.rail__note{font:400 11px/1.5 var(--sans);color:var(--subtle);padding:2px 4px;}

/* ---- main ---- */
main{min-width:0;padding:0 clamp(24px,4vw,64px) 160px;}
.wrap{max-width:var(--measure);margin:0 auto;}

.masthead{padding:76px 0 34px;border-bottom:1px solid var(--line);}
.kicker{font:500 11px/1 var(--mono);letter-spacing:.16em;text-transform:uppercase;color:var(--accent);margin-bottom:20px;}
.masthead h1{font:585 clamp(34px,5vw,60px)/1.02 var(--display);letter-spacing:-.02em;
  font-optical-sizing:auto;margin:0;text-wrap:balance;}
.masthead .lede{margin:22px 0 0;max-width:66ch;font:400 18px/1.55 var(--sans);color:var(--muted);}
.masthead .lede em{font-family:var(--display);font-style:italic;color:var(--ink);font-weight:500;}
.byline{margin-top:20px;font:400 12.5px/1.6 var(--mono);color:var(--subtle);}
.byline b{color:var(--muted);font-weight:500;}

.statline{display:grid;grid-template-columns:repeat(auto-fit,minmax(118px,1fr));gap:1px;
  margin:36px 0 0;background:var(--line);border:1px solid var(--line);border-radius:12px;overflow:hidden;}
.statline>div{background:var(--surface);padding:16px 16px;}
.statline .v{font:300 30px/1 var(--display);letter-spacing:-.01em;font-variant-numeric:tabular-nums;color:var(--ink);}
.statline .v.accent{color:var(--accent);}
.statline .k{font:500 9.5px/1.4 var(--mono);letter-spacing:.1em;text-transform:uppercase;color:var(--subtle);margin-top:9px;}

/* ---- doc section ---- */
.doc{padding:56px 0 20px;border-bottom:1px solid var(--line-soft);scroll-margin-top:8px;position:relative;}
.doc__head{margin-bottom:26px;}
.doc__meta{display:flex;align-items:center;gap:10px;flex-wrap:wrap;margin-bottom:12px;}
.doc__num{font:500 11px/1 var(--mono);letter-spacing:.1em;color:var(--accent);}
.doc__tag{font:500 9.5px/1.4 var(--mono);letter-spacing:.11em;text-transform:uppercase;color:var(--subtle);
  border:1px solid var(--line-strong);border-radius:5px;padding:3px 7px;}
.doc__file{font:400 11.5px/1 var(--mono);color:var(--subtle);margin-left:auto;
  background:var(--fill);border-radius:5px;padding:4px 8px;}
.doc__title{font:560 30px/1.15 var(--display);letter-spacing:-.015em;color:var(--ink);margin:0;text-wrap:balance;}
.doc__top{position:absolute;top:56px;right:0;font:500 10.5px/1 var(--mono);letter-spacing:.06em;
  color:var(--subtle);opacity:0;transition:opacity .15s;}
.doc:hover .doc__top{opacity:1;}
.doc__top:hover{color:var(--accent);text-decoration:none;}

/* ---- prose ---- */
.doc__body{max-width:100%;}
.doc__body>*{max-width:74ch;}
.doc__body .tablewrap,.doc__body pre{max-width:100%;}
.doc__body h1,.doc__body h2,.doc__body h3,.doc__body h4{
  font-family:var(--display);letter-spacing:-.01em;color:var(--ink);line-height:1.25;
  scroll-margin-top:12px;text-wrap:balance;}
.doc__body h1{font-size:26px;font-weight:560;margin:44px 0 14px;}
.doc__body h2{font-size:21px;font-weight:560;margin:40px 0 12px;padding-top:8px;border-top:1px solid var(--line-soft);}
.doc__body h2:first-child{border-top:0;padding-top:0;margin-top:6px;}
.doc__body h3{font-size:16.5px;font-weight:600;font-family:var(--sans);margin:28px 0 8px;letter-spacing:-.005em;}
.doc__body h4{font-size:12.5px;font-weight:600;font-family:var(--mono);letter-spacing:.06em;
  text-transform:uppercase;color:var(--muted);margin:22px 0 8px;}
.doc__body p{margin:0 0 14px;}
.doc__body ul,.doc__body ol{margin:0 0 16px;padding-left:24px;}
.doc__body li{margin:5px 0;}
.doc__body li>ul,.doc__body li>ol{margin:6px 0;}
.doc__body strong{font-weight:640;color:var(--ink);}
.doc__body em{font-style:italic;}
.doc__body hr{border:0;border-top:1px solid var(--line);margin:32px 0;}
.doc__body blockquote{margin:18px 0;padding:2px 0 2px 18px;border-left:2px solid var(--accent-bar);
  color:var(--muted);font-style:italic;}
.doc__body blockquote p:last-child{margin-bottom:0;}

/* inline + block code */
.doc__body code{font:450 .86em/1.5 var(--mono);background:var(--fill);
  border-radius:4px;padding:1.5px 5px;color:var(--ink);word-break:break-word;}
.doc__body pre{background:var(--code-bg);border:1px solid var(--line);border-radius:12px;
  padding:14px 16px;overflow-x:auto;margin:16px 0;line-height:1.6;}
.doc__body pre code{background:transparent;padding:0;border-radius:0;font-size:12.8px;color:var(--code-ink);
  white-space:pre;word-break:normal;}

/* tables */
.tablewrap{overflow-x:auto;margin:18px 0;border:1px solid var(--line);border-radius:10px;
  -webkit-overflow-scrolling:touch;}
.doc__body table{border-collapse:collapse;width:100%;font-size:13px;background:var(--surface);}
.doc__body thead th{position:sticky;top:0;text-align:left;font:600 10px/1.4 var(--mono);
  letter-spacing:.06em;text-transform:uppercase;color:var(--subtle);background:var(--fill);
  padding:10px 13px;border-bottom:1px solid var(--line-strong);white-space:nowrap;}
.doc__body tbody td{padding:9px 13px;border-bottom:1px solid var(--line-soft);
  vertical-align:top;color:var(--ink);}
.doc__body tbody tr:last-child td{border-bottom:0;}
.doc__body tbody tr:hover{background:var(--fill);}
.doc__body td code,.doc__body th code{font-size:11.6px;}

/* ---- floating progress + reduced motion ---- */
.progress{position:fixed;top:0;left:var(--rail);right:0;height:2px;background:transparent;z-index:50;}
.progress__bar{height:100%;width:0;background:var(--accent-bar);transition:width .1s linear;}
@media (prefers-reduced-motion: reduce){*{animation:none!important;transition:none!important;}html{scroll-behavior:auto;}}

/* reveal on load */
.doc{opacity:0;transform:translateY(8px);animation:rise .5s cubic-bezier(.2,0,0,1) forwards;}
@keyframes rise{to{opacity:1;transform:none;}}

@media (max-width:940px){
  .shell{grid-template-columns:1fr;}
  .rail{position:static;height:auto;border-right:0;border-bottom:1px solid var(--line);}
  .progress{left:0;}
  main{padding:0 20px 100px;}
}
"""

JS = r"""
(function(){
  var root=document.documentElement, KEY='br-review-theme';
  var btns=[].slice.call(document.querySelectorAll('[data-theme-set]'));
  function apply(m){ if(m==='system')root.removeAttribute('data-theme'); else root.setAttribute('data-theme',m);
    btns.forEach(function(b){b.setAttribute('aria-pressed',String(b.getAttribute('data-theme-set')===m));});
    try{localStorage.setItem(KEY,m);}catch(e){} }
  btns.forEach(function(b){b.addEventListener('click',function(){apply(b.getAttribute('data-theme-set'));});});
  var saved='system'; try{saved=localStorage.getItem(KEY)||'system';}catch(e){} apply(saved);

  // scrollspy
  var links=[].slice.call(document.querySelectorAll('.rail__link'));
  var map={}; links.forEach(function(a){map[a.getAttribute('href').slice(1)]=a;});
  var secs=[].slice.call(document.querySelectorAll('.doc'));
  var active=null;
  function setActive(a){ if(a===active)return; if(active)active.classList.remove('is-active');
    active=a; if(a){a.classList.add('is-active'); if(a.scrollIntoViewIfNeeded)a.scrollIntoViewIfNeeded();} }
  if('IntersectionObserver' in window){
    var seen={};
    var io=new IntersectionObserver(function(es){
      es.forEach(function(e){seen[e.target.id]=e;});
      var best=null;
      Object.keys(seen).forEach(function(id){var e=seen[id];
        if(e.isIntersecting){ if(!best||e.boundingClientRect.top<best.boundingClientRect.top)best=e;}});
      if(best&&map[best.target.id])setActive(map[best.target.id]);
    },{rootMargin:'-8% 0px -80% 0px',threshold:0});
    secs.forEach(function(s){io.observe(s);});
  }
  // reading progress
  var bar=document.querySelector('.progress__bar');
  function onScroll(){var h=document.documentElement;
    var p=h.scrollTop/((h.scrollHeight-h.clientHeight)||1); if(bar)bar.style.width=(p*100).toFixed(1)+'%';}
  document.addEventListener('scroll',onScroll,{passive:true}); onScroll();
})();
"""

MARK_SVG = ('<svg class="brand__mark" viewBox="0 0 32 32" fill="none" aria-hidden="true">'
  '<rect x="1" y="1" width="30" height="30" rx="8" fill="var(--accent)"/>'
  '<path d="M10 22V10h5.2a3.4 3.4 0 0 1 0 6.8H10" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>'
  '<path d="M15.4 16.8 21 22" stroke="#fff" stroke-width="2" stroke-linecap="round"/>'
  '<circle cx="21.6" cy="10.4" r="1.9" fill="#fff"/></svg>')

page = f"""<!DOCTYPE html>
<html lang="en"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>BioRouter Agentic Loop Review</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Fraunces:ital,opsz,wght@0,9..144,400;0,9..144,500;0,9..144,560;0,9..144,600;1,9..144,400;1,9..144,500&family=JetBrains+Mono:wght@400;500;600&display=swap" rel="stylesheet">
<style>{CSS}</style>
</head>
<body>
<a id="top"></a>
<div class="progress"><div class="progress__bar"></div></div>
<div class="shell">
<aside class="rail">
  <div class="brand">{MARK_SVG}<div><div class="brand__name">BioRouter</div><div class="brand__sub">Agentic&nbsp;Loop&nbsp;Review</div></div></div>
  <nav class="rail__nav" aria-label="Documents">
    <a class="rail__link" href="#top"><span class="rail__t">Cover</span></a>
{nav_html}
  </nav>
  <div class="rail__foot">
    <div class="themetoggle" role="group" aria-label="Theme">
      <button type="button" data-theme-set="light" aria-pressed="false">Light</button>
      <button type="button" data-theme-set="dark" aria-pressed="false">Dark</button>
      <button type="button" data-theme-set="system" aria-pressed="true">Auto</button>
    </div>
    <div class="rail__note">28 documents · 27 agents · generated from the review corpus.</div>
  </div>
</aside>
<main><div class="wrap">
<header class="masthead">
  <div class="kicker">Internal review · v1.87.2 · {len(DOCS)} documents</div>
  <h1>How the agent thinks, and every way to make it think better.</h1>
  <p class="lede">A multi-agent audit of BioRouter's agentic feedback loop — <em>context, tools, compaction, hooks, guardrails, loops, processes and verification</em> — benchmarked against nine open-source coding agents and distilled into a prioritized program of 67 improvements.</p>
  <div class="byline"><b>Scope:</b> 10 internal subsystem reviews · 9 external tool studies · 4 comparison chapters · 3 proposal lenses &nbsp;→&nbsp; <b>1</b> executive report + <b>67</b> proposals</div>
  <div class="statline">
    <div><div class="v accent">67</div><div class="k">Proposals</div></div>
    <div><div class="v">10</div><div class="k">Internal reviews</div></div>
    <div><div class="v">9</div><div class="k">Agents compared</div></div>
    <div><div class="v">4</div><div class="k">Comparison chapters</div></div>
    <div><div class="v">3</div><div class="k">Proposal lenses</div></div>
    <div><div class="v">28</div><div class="k">Documents</div></div>
  </div>
</header>
{sec_html}
</div></main>
</div>
<script>{JS}</script>
</body></html>
"""

OUT.write_text(page, encoding="utf-8")
print(f"wrote {OUT} ({len(page):,} bytes) from {len(DOCS)} docs")
