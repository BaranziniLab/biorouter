# Third-party licences for the vendored Auto Visualiser assets

Every file in this directory is a third-party library. Each one is compiled into
the BioRouter binary with `include_str!` (see `../../common.rs`) and inlined into
the HTML of every figure the Auto Visualiser generates, so BioRouter redistributes
all of them to every user. MIT, BSD and ISC each require the copyright notice and
the licence text to travel with a redistributed copy. This file is that notice.

Generated figures carry a short HTML comment naming these libraries and pointing
back here. The comment is built from `common::ATTRIBUTION_COMMENT`.

If you add, remove or upgrade a file in this directory, update the matching entry
below. `every_vendored_asset_is_covered_by_the_licence_file` in
`../../tests.rs` fails when a file has no entry, or an entry names no file.

Each version below was confirmed against the library's own published release, not
read off a banner alone: `chart.min.js`, `leaflet.min.js`,
`leaflet.markercluster.min.js` and `mermaid.min.js` are byte-for-byte identical to
the upstream build, and `d3.min.js`, `d3.sankey.min.js` and `leaflet.min.css`
differ from it only in leading indentation.

## Contents

| File | Library | Version | Licence |
|------|---------|---------|---------|
| `chart.min.js` | Chart.js | 4.5.0 | MIT |
| `d3.min.js` | D3 | 7.9.0 | ISC |
| `d3.sankey.min.js` | d3-sankey | 0.12.3 | BSD-3-Clause |
| `leaflet.min.js` | Leaflet | 1.9.4 | BSD-2-Clause |
| `leaflet.min.css` | Leaflet | 1.9.4 | BSD-2-Clause |
| `leaflet.markercluster.min.js` | Leaflet.markercluster | 1.5.3 | MIT |
| `mermaid.min.js` | Mermaid | 11.17.2 | MIT |

---

## `chart.min.js`

- **Library:** Chart.js
- **Version:** 4.5.0 (upstream `chart.js@4.5.0/dist/chart.umd.min.js`)
- **Licence:** MIT
- **Copyright:** Copyright (c) 2014-2024 Chart.js Contributors. The banner inside
  the minified file reads `(c) 2025 Chart.js Contributors`.
- **Project:** https://github.com/chartjs/Chart.js

```
The MIT License (MIT)

Copyright (c) 2014-2024 Chart.js Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

---

## `d3.min.js`

- **Library:** D3
- **Version:** 7.9.0 (upstream `d3@7.9.0/dist/d3.min.js`)
- **Licence:** ISC
- **Copyright:** Copyright 2010-2023 Mike Bostock
- **Project:** https://github.com/d3/d3

```
Copyright 2010-2023 Mike Bostock

Permission to use, copy, modify, and/or distribute this software for any purpose
with or without fee is hereby granted, provided that the above copyright notice
and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH
REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND
FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT,
INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS
OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER
TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF
THIS SOFTWARE.
```

---

## `d3.sankey.min.js`

- **Library:** d3-sankey
- **Version:** 0.12.3 (upstream `d3-sankey@0.12.3/dist/d3-sankey.min.js`)
- **Licence:** BSD-3-Clause
- **Copyright:** Copyright 2015, Mike Bostock
- **Project:** https://github.com/d3/d3-sankey

```
Copyright 2015, Mike Bostock
All rights reserved.

Redistribution and use in source and binary forms, with or without modification,
are permitted provided that the following conditions are met:

* Redistributions of source code must retain the above copyright notice, this
  list of conditions and the following disclaimer.

* Redistributions in binary form must reproduce the above copyright notice,
  this list of conditions and the following disclaimer in the documentation
  and/or other materials provided with the distribution.

* Neither the name of the author nor the names of contributors may be used to
  endorse or promote products derived from this software without specific prior
  written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

---

## `leaflet.min.js`

- **Library:** Leaflet
- **Version:** 1.9.4 (upstream `leaflet@1.9.4/dist/leaflet.js`)
- **Licence:** BSD-2-Clause
- **Copyright:** Copyright (c) 2010-2023, Volodymyr Agafonkin; Copyright (c)
  2010-2011, CloudMade
- **Project:** https://github.com/Leaflet/Leaflet

```
BSD 2-Clause License

Copyright (c) 2010-2023, Volodymyr Agafonkin
Copyright (c) 2010-2011, CloudMade
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

---

## `leaflet.min.css`

- **Library:** Leaflet (the stylesheet half of the same release)
- **Version:** 1.9.4 (upstream `leaflet@1.9.4/dist/leaflet.css`)
- **Licence:** BSD-2-Clause
- **Copyright:** Copyright (c) 2010-2023, Volodymyr Agafonkin; Copyright (c)
  2010-2011, CloudMade
- **Project:** https://github.com/Leaflet/Leaflet

```
BSD 2-Clause License

Copyright (c) 2010-2023, Volodymyr Agafonkin
Copyright (c) 2010-2011, CloudMade
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

---

## `leaflet.markercluster.min.js`

- **Library:** Leaflet.markercluster
- **Version:** 1.5.3 (upstream
  `leaflet.markercluster@1.5.3/dist/leaflet.markercluster.js`, which is the
  minified build despite the name)
- **Licence:** MIT
- **Copyright:** Copyright 2012 David Leaver
- **Project:** https://github.com/Leaflet/Leaflet.markercluster

```
Copyright 2012 David Leaver

Permission is hereby granted, free of charge, to any person obtaining
a copy of this software and associated documentation files (the
"Software"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to
the following conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE
LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

---

## `mermaid.min.js`

- **Library:** Mermaid
- **Version:** 11.17.2 (upstream `mermaid@11.17.2/dist/mermaid.min.js`). This is
  the same release `CDN_MERMAID` in `../../common.rs` pins, so CDN mode and the
  vendored offline copy are the same bytes.
  `vendored_and_cdn_mermaid_are_the_same_version` in `../../tests.rs` reads the
  version back out of this file's bundle and fails if the two ever diverge.
- **Licence:** MIT
- **Copyright:** Copyright (c) 2014 - 2022 Knut Sveidqvist
- **Project:** https://github.com/mermaid-js/mermaid

```
The MIT License (MIT)

Copyright (c) 2014 - 2022 Knut Sveidqvist

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
