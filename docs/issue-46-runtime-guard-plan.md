# Issue #46 runtime guard plan

This temporary implementation note records the runtime wiring for speaker-profile consistency.

- `speaker_profile::resolve_speaker_profiles` is the single online resolver.
- `doctor` reports profile mismatches before generation.
- `generate` resolves profiles before parsing/validation and fails on any profile error.
- Character-linked profiles remain authoritative by named VOICEVOX speaker/style.
- Legacy numeric-only profiles are surfaced as warnings, not silently treated as matching the script-visible name.
- The runtime never rewrites user configuration implicitly.

This note can be folded into the permanent docs once #46 is complete.
