# Historical intermediate-build evidence — not current acceptance

These original logs are retained unchanged. They predate schema 2 build provenance.
The six fresh/reused-profile runs have no reply kind or localImageLoaded field.
final-communication.jsonl has reply kinds but no localImageLoaded field.
acceptance.jsonl includes both, but still has no embedded commit/build identity.
Their exact binary identities were not captured and cannot be reconstructed from
metrics alone. They must not certify the current source or pass verify_run.py.
Use them only for the explicitly historical observations in the report. Do not
backfill fields or label untyped replies as ping-only. New versioned evidence is
stored separately and supersedes these logs for the automated acceptance subset.
