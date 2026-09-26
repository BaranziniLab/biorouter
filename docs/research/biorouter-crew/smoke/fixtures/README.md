# Synthetic CSV summarizer fixture

`summarize_csv.py` is a test-only fixture copied byte-for-byte from the verified local and Carol SSH checks. Its SHA-256 is:

```text
901605725a653a05544cd83bb7af79f711321475fb6d812a91cb991cc7f544d6
```

The synthetic input is:

```csv
sample_id,value
A,10
B,20
C,30
```

Run it from the directory containing `crew-task.csv` with the exact agent command:

```text
["python3", "summarize_csv.py", "crew-task.csv", "summary.csv"]
```

The expected computed output is:

```csv
row_count,total
3,60
```

For the verified input, this output is 21 bytes with SHA-256 `29f857b262032224f20e849e76bed6907e8e02846835a632c8380b2bafb91ab3`.

This fixture contains only synthetic data and code. It has no credentials or session state. Its output is a local or remote fixture check; it is separate from any UI-produced `summary.csv` and cannot establish graphical workflow success.
