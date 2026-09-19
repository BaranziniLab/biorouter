# Web & Documents capability

Web & Documents (`webdocuments`) supplies five built-in tools independently of the native desktop runtime:

| Tool | Purpose |
| --- | --- |
| `web_scrape` | Fetch a known HTTP(S) URL, return content, and cache the response. |
| `xlsx_tool` | Read and update Excel workbooks. |
| `docx_tool` | Extract, create, and update Word documents. |
| `pdf_tool` | Extract PDF text and images. |
| `cache` | List, view, delete, and clear cached files. |

Enable it in Settings → Chat → Capabilities or `biorouter configure`. Use the advertised schemas and existing file/privacy permissions. Fetching a known URL does not require desktop control or computer-use approval. Document and cache mutations retain their ordinary approval rules.

The tools now use `webdocuments__` names (or the `webdocuments` JavaScript module). The former Computer Use names do not forward. [Computer Use](computer-controller.md) handles native application interaction and screenshots; [Developer](developer.md) handles code and ordinary files.
