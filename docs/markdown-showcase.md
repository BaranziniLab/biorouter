# A clear view of the results

Use this synthetic template to preview the same markdown in a chat and in a document. Replace the example findings with your own; retain the structure as needed.

## Findings at a glance

A useful report begins with **one concrete finding**, followed by evidence. Inline `sample_id` values stay readable, and [external documentation](https://example.com) remains easy to identify.

| Measure                        |          Baseline |         Follow-up | Interpretation                     |
| :----------------------------- | ----------------: | ----------------: | :--------------------------------- |
| Samples                        |               128 |               144 | Synthetic counts for layout review |
| α coefficient                  |         5.809e-04 |         0.0005809 | Full precision remains visible     |
| Mean response                  |             12.50 |             10.75 | A modest reduction                 |
| Long_identifier_with_no_breaks | 1234567890.012345 | 9876543210.543210 | Scroll the table in a narrow panel |

> **Interpretation**
>
> These are illustrative values. This callout separates an interpretation from the underlying table without interrupting the reading flow.

## Reproduce the calculation

```python
values = [12.50, 10.75]
change = values[1] - values[0]
print(f"Change: {change:.2f}")
```

### Plain text output

```
Sample       Result
control      12.50
follow-up    10.75
Δ            −1.75
```

### Mathematical notation

The standardized difference is $$d = (\bar{x}_1 - \bar{x}_2)/s$$ within a sentence.

$$
\hat{\mu} = \frac{1}{n}\sum_{i=1}^{n} x_i
$$

## Review checklist

1. Check the input.
   - Confirm labels and units.
   - Preserve Unicode: 中文、日本語、α, β, Δ, and café.
2. Review the output.
   - Compare the table with the calculation.
     - Keep the original precision.
     - Explain any exclusions.
3. Save the report and record the next step.

#### Notes

- [x] Add an informative title.
- [x] Keep comparison columns aligned.
- [ ] Replace synthetic values with verified findings.

---

A final paragraph provides context and a next step without repeating every detail above.
