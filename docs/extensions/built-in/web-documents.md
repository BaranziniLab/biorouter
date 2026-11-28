# Web & Documents capability

Web & Documents (`webdocuments`) supplies five built-in tools independently of the native desktop runtime:

| Tool | Purpose |
| --- | --- |
| `web_scrape` | Fetch a known HTTP(S) URL, return content, and cache the response. |
| `xlsx_tool` | Read and update Excel workbooks. |
| `docx_tool` | Extract, create, and update Word documents. |
| `pdf_tool` | Extract PDF text and images. |
| `cache` | List, view, delete, and clear cached files. |

Enable it in Settings → Chat → Capabilities or `biorouter configure`. Use the advertised schemas and existing file/privacy permissions. Fetching a known URL does not require desktop control or Biorouter Copilot approval. Document and cache mutations retain their ordinary approval rules.

The tools now use `webdocuments__` names (or the `webdocuments` JavaScript module). The former `computercontroller__` names do not forward. [Biorouter Copilot](computer-controller.md) handles native application interaction and screenshots; [Developer](developer.md) handles code and ordinary files.

## Which addresses `web_scrape` reaches

The model chooses the URL `web_scrape` fetches, so it reaches public internet addresses only. Before anything is sent, it refuses:

- a scheme other than `http` or `https`;
- `localhost`, names under `.localhost`, `.local`, `.internal` and `.home.arpa`, a single-label name such as `intranet`, and cloud metadata names such as `metadata.google.internal`;
- an address on this computer or a private network: loopback, private (RFC 1918), link-local, carrier-grade NAT (`100.64.0.0/10`), unique-local and other non-global IPv6, IPv4-mapped IPv6, multicast, unspecified, and cloud metadata addresses such as `169.254.169.254` and `fd00:ec2::254`.

A host name is resolved once, and every address it resolves to is checked. The connection goes to those checked addresses, so a DNS answer that changes after the check cannot send it anywhere else. Every redirect hop is checked the same way, up to 10 hops. The fetch never goes through a proxy (`HTTP_PROXY`, `HTTPS_PROXY` or a system proxy), because a proxy resolves the address itself and nothing could check it.

Knowledge ingest from a URL and the knowledge credibility check's page fetch follow the same rule; all three use `crates/biorouter-mcp/src/safe_fetch.rs`. To read a page on this computer or a private network, save it and attach the file.
