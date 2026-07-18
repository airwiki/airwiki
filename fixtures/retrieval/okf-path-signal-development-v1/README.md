# OKF path-signal development fixture v1

This synthetic fixture measures whether reviewed OKF links contain useful
navigation signal beyond a weak-degree-preserving structural sham. It is a
development diagnostic, not a production-promotion corpus.

The artifacts were authored with procedural separation:

1. the concept author produced `concepts.json` without links or pair labels;
2. the link author saw only the sealed concept inventory and produced
   `links.json` without pair labels or evaluation results;
3. the pair author saw only the sealed concept inventory and produced
   `cases.json` without links, graph topology or evaluation results; and
4. the integrator created `manifest.json` after all three artifacts were
   frozen. No artifact may be edited in response to observed metrics.

This procedure limits direct leakage between topology and labels. It is not a
claim of statistical independence. All documents and domains are fictional.

The evaluator treats a bidirectional path of at most two reviewed-link hops as
a navigation chain. It is not logical entailment, causal proof or documentary
evidence. Intermediate nodes never become search hits or citations. A budget
exhaustion or unavailable endpoint is indeterminate and vetoes the gate rather
than being counted as a negative.

Run the frozen diagnostic in a release build:

```bash
cargo run --release --locked -p xtask -- retrieval evaluate-path-signal
```

Passing the development gate can authorize a separate experiment that uses
the graph to reorder an already authorized hybrid candidate set. It cannot
authorize a production change by itself, and
`production_promotion_ready` remains `false` in every report.
