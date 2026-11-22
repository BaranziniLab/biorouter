"""Biorouter BRXT adapter kit, runtime half.

This package is vendored, byte for byte, into every kit adapter as
``src/<package>/_kit/``. Do not edit a vendored copy: change the kit in the
biorouter repository (``scripts/brxt-adapter-kit/brxt_kit/``), then run
``bin/vendor_kit.py`` against the adapter. The adapter's tests compare the
vendored files with the hashes in ``_kit/vendored.json`` and fail on any edit.

Modules:

* ``safe_http``: the only HTTP path an adapter may use. Every credential is
  bound to the origin of its own target and is never sent anywhere else.
* ``spec``: reads and validates an adapter's ``adapter.json``.
* ``tools``: the three tool shapes (``<svc>_status``, ``<svc>_get``,
  ``<svc>_write``) and the sunset and tombstone servers.
"""

KIT_VERSION = "1.0.1"

__all__ = ["KIT_VERSION"]
