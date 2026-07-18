# Experiment evidence

The one-shot runner writes `compatibility-attempt.json` and, when it completes,
`compatibility-report.json` here. These aggregate files are committed with the
experiment outcome. An existing receipt blocks rerunning the same experiment
version, including after a crash or failed diagnostic.

No document text, query text, passage text, individual corpus identifier, label
or per-pair score belongs in this directory.
