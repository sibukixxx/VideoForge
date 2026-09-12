# Managed VOICEVOX lifecycle proposal

## Decision

VideoForge Desktop should eventually offer automatic VOICEVOX Engine startup, but it should not
blindly start or kill a process whenever the window opens. Use an opt-in managed lifecycle:

```text
Desktop starts
  -> probe configured endpoint
  -> already healthy: reuse external engine, never stop it
  -> unavailable + managed engine configured: spawn engine
  -> wait with timeout/backoff until healthy
  -> expose status to UI

Desktop exits
  -> stop only the child process started by this VideoForge instance
```

This keeps the existing HTTP `VoicevoxEngine` adapter unchanged. Process ownership belongs in the
desktop composition root, not `videoforge-core` and not the Wasm module.

## Proposed configuration

Do not add these keys until the packaging path is chosen and tested on both macOS and Windows.

```yaml
tts:
  engine: voicevox
  endpoint: http://127.0.0.1:50021
  managed:
    enabled: true
    executable: auto
    startup_timeout_seconds: 30
```

`auto` should search an explicitly supported installation or bundled sidecar location. It must not
scan arbitrary directories or execute an untrusted binary found on `PATH` without showing which
binary will run.

## Required states

- `external_running`: endpoint was already healthy; VideoForge does not own the process.
- `starting`: managed child was spawned and health polling is in progress.
- `managed_running`: VideoForge owns the child and may stop it on exit.
- `unavailable`: no healthy endpoint and no configured executable.
- `failed`: spawn failed, startup timed out, or the child exited early.

The UI should show the state, selected executable, endpoint, and a retry/start button. Generation
must still return the existing `voicevox_unavailable` error when no engine becomes healthy.

## Safety and lifecycle rules

- Bind to loopback by default and keep the existing remote-endpoint opt-in.
- Detect endpoint health before spawning to avoid port conflicts.
- Store the owned child handle in Tauri `AppState`.
- Never terminate a pre-existing external VOICEVOX process.
- On normal exit, request graceful termination and then use a bounded forced shutdown only for the
  owned child.
- Surface child stderr without placing arbitrary command output into user-facing HTML.
- If the child crashes, clear ownership and allow an explicit retry.
- Prevent two simultaneous startup attempts with a mutex/state machine.

## Delivery phases

1. **Discovery spike:** document supported VOICEVOX executable locations and command arguments on
   the actual macOS and Windows installations used for VideoForge.
2. **External executable mode:** user selects an installed engine executable; VideoForge manages
   only that child. This avoids installer size and redistribution questions.
3. **Optional bundled sidecar:** only after redistribution terms, update flow, binary size,
   architecture variants, and signing/notarization are confirmed.

Phase 2 is the recommended first implementation. Bundling the entire engine should not be assumed
to be the default simply for convenience.
