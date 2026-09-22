# Crew SSH hop policy

Crew uses native OpenSSH configuration, identities and interactive authentication. Before authentication and bridge startup, it resolves the effective final invocation and every implicit ProxyJump child with `ssh -G`. It checks each hop independently: final-host command-line options do not propagate to jump hosts.

A jump alias may need this stanza **before broader matching defaults** in the user's or site's SSH config:

```sshconfig
Host crew-gateway
    StrictHostKeyChecking yes
    ForwardAgent no
    ForwardX11 no
    PermitLocalCommand no
    ClearAllForwardings yes
    NoHostAuthenticationForLocalhost no
    GSSAPIDelegateCredentials no
    Tunnel no
    ForkAfterAuthentication no
    ControlMaster no
    ControlPath none
    ControlPersist no
```

Keep the existing HostName, User, Port, IdentityFile and interactive authentication settings. Identity-agent authentication remains available; forwarding that agent is forbidden. Pin host keys using the normal verified known-hosts process. Crew does not automatically accept unknown keys. An unsupported GSSAPI option may be omitted on clients without that feature. Older OpenSSH clients without the ForkAfterAuthentication config keyword may omit it; Crew never supplies their equivalent `-f` flag.

Custom ProxyCommand routes are refused; use native ProxyJump. Each implicit child is inspected with its actual user/port overrides, remaining jump chain, inherited `-F`, and `-W` destination. Cycles and routes exceeding sixteen hosts are refused. Shell-sensitive jump syntax or inherited configuration paths are refused with remediation rather than passed unchecked to OpenSSH's internally constructed shell command. The admitted inherited `-F` grammar also rejects backslashes, including an ordinary Windows path such as `C:\crew\config`, even when it contains no spaces. Native Windows configuration-path quoting and ProxyJump behavior remain unqualified; this implementation must not be described as supporting those routes merely because their paths lack spaces.

Preflight trusts configuration maintained by the same user and site administrator, including native Match/Include behavior. It is not a sandbox for malicious configuration or concurrent edits between inspection and connection. Configuration is neither copied nor reconstructed from `ssh -G` output. Output is bounded and never included in model-visible errors. Authentication and bridge startup each repeat the check. Crew's final master remains explicitly owned by its authentication flow; jump hosts cannot reuse unrelated masters. Use the explicit Close control to end the authenticated connection.

Implementation references: [OpenSSH implicit jump invocation](https://github.com/openssh/openssh-portable/blob/master/ssh.c), [native configuration parser and diagnostic dump](https://github.com/openssh/openssh-portable/blob/master/readconf.c).
