# A better Markdown preview

Readable documents with **real bold**, *italic*, ***both together***,
~~strikethrough~~ and `inline code`. A [reference link](https://example.org)
stays inert: no network requests.

## Tables that behave like tables

| Feature | Status | Count |
| :------ | :----: | ----: |
| **Native rendering** | Ready | 12 |
| A longer value wraps inside its cell without disrupting the other columns | Tested | 345 |

## Lists and tasks

3. Keep ordered numbers
   - Nest a bullet
     - And another level
4. Preserve paragraphs within an item.

   This is still the same item, not a new bullet.

- [x] Support checked tasks
- [ ] Keep unchecked tasks read-only

> A quote is its own document container.
>
> Second paragraph with **emphasis**.
>
> > Nested quotation.

### Highlighted code

```rust
// Highlighted on the preview worker
fn main() {
    let message = "Hello, Markdown!";
    println!("{message}");
}
```

Hard line break here.\
This is the next line.

---

![Image description only](https://example.org/no-fetch.png)
