/-!
# Specification: EPUB package manifest, spine resolution and container paths (G12)

Pure Lean over `Nat`, `Bool` and `List`: what the shipped `src/epub.rs` must compute when it
turns an EPUB's package document into the list of chapters it reads, and, as proved
theorems, what it must never do. Written before the implementation and not shaped to it;
nothing here imports an extraction. Every declaration rests on the kernel alone (no
`propext`, `Quot.sound` or `Classical.choice`); `proof/audit.py` checks this with an empty
allowlist.

Sources (EPUB 3.3, W3C Recommendation 2023-05-25; OCF = its "Open Container Format" part;
URL = WHATWG URL Standard; RFC 3986):
* EPUB 3.3 "Package document" (the `package`, `manifest`, `item`, `spine` and `itemref`
  elements): `package` is the root; `manifest` and `spine` are its children; the manifest
  contains `item` elements with the required attributes `id`, `href` and `media-type`; the
  spine contains `itemref` elements whose required `idref` must reference the id of a
  manifest item, and the spine order is the default reading order. All of these are in
  the OPF namespace `http://www.idpf.org/2007/opf`.
* XML 1.0 3.3.1 (Validity constraint "ID"): "Values of type ID MUST match the Name
  production", so an id is non-empty, and "a name MUST NOT appear more than once in an
  XML document as a value of this type": two manifest items with one id are an error, and
  neither of them is "the" item that id names.
* EPUB 3.3 `itemref` (each itemref references a unique manifest item; epubcheck reports a
  second reference to the same item as OPF-034) and spine items (each referenced item is
  an EPUB content document, XHTML `application/xhtml+xml` or SVG `image/svg+xml`, or
  foreign content with a fallback chain); foreign content is not text to extract.
* EPUB 3.3 "Open Container Format" (OCF: the root directory, the container root URL, file
  paths and URLs in the OCF abstract container, `container.xml` `rootfile@full-path`):
  `href` and `full-path` are URL strings; a relative `href` is resolved against the URL of
  the package document, and the root directory is the root of resolution, so a
  path-absolute `href` (`/x`) names `x` in the root directory whatever the package
  directory. Resolution removes dot segments (RFC 3986 5.2.4: `.` segments vanish, `..`
  removes the preceding segment; as in the URL parser, `..` at the root stays at the root).
  Query (`?`) and fragment (`#`) are not part of the path. The path is percent-decoded
  (URL Standard 1.3 "percent-decode": `%` followed by two ASCII hex digits is the byte they
  denote; any other `%` is kept as it is) to obtain the ZIP entry name; decoding happens
  after the path is split into segments, so `%2F` never becomes a separator, and a segment
  that decodes to `.` or `..` is a dot segment (RFC 3986 6.2.2.2 decodes unreserved
  octets, `.` among them, before dot-segment removal).

Deliberate readings:
* An id that no item has resolves to `missing`; an id that two or more items have resolves
  to `duplicate`. Neither yields an item; both are reported, so a spine is never silently
  shortened and never reads an item it did not name.
* A spine reference whose item is foreign content is reported as `foreign` and not read
  (no fallback chain is followed). A second reference to an item already read is reported
  as `repeated` and not read again.
* The rendered text is refused when the joined chapters (with the two-byte `\n\n`
  separator between chapters) exceed the budget; nothing partial is returned. A ZIP entry
  is accepted only if both its declared size and the bytes actually read are within the
  entry bound.

The manifest index. Production keeps the id lookup in a table (the apeiron `Map`/`Table`
shape, whose canonical object is a function from key to value): one entry per distinct id
holding `(first position, duplicated)`, written by one insert per manifest item in document
order. `Index`, `buildIndex` and `lookupIndex` model exactly that; `lookupIndex_build`
proves the table answers every id as the declarative `resolveId`. `spine_step` also checks
that the position the table returns is in range and that the item stored there has the
idref's id (`spineCode_read`), so even a wrong table answer can only produce a refusal,
never a wrong chapter; `codeAt_eq` proves that on a correctly built table the kernel
decides exactly as `spineStep`.

Kernels. Each decision is a loop-free kernel over `bool`/`u8`/`u32`/`u64` that production
calls (to live in `proof/extract_epub.rs`, included by `src/epub.rs` through `#[path]`);
the refinement module proves each equal to its field function here. The loops stay in Rust
and are exactly the recursions `roleFrom`, `buildFrom`, `spineGo`, `pathPart`, `splitGo`,
`pctGo`, `removeDotsGo` and `renderGo` below.
* `opf_child_role(parent: u8, name: u8, opf_ns: bool) -> u8` is `childRole`.
* `manifest_item_accept(has_id: bool, has_href: bool, id_len: u64) -> bool` is `itemAccept`.
* `manifest_index_step(present: bool, prev_index: u32, k: u32) -> (u32, bool)` is
  `indexFields`.
* `spine_step(found: bool, dup: bool, index: u32, count: u32, id_eq: bool, content_doc:
  bool, seen: bool) -> u8` is `spineCode`.
* `href_byte_class(b: u8) -> u8` is `byteClass`.
* `pct_hex(b: u8) -> (bool, u8)` is `hexFields`.
* `path_segment_kind(len: u64, b0: u8, b1: u8) -> u8` is `segKindFields`.
* `epub_text_budget(out_len: u64, sep_len: u64, md_len: u64, max: u64) -> (bool, u64)` is
  `textBudget`.
* `zip_entry_bound(declared: u64, read: u64, max: u64) -> bool` is `entryBoundOk`.
-/

namespace EpubSpec

/-! ## Helpers without `propext` -/

theorem decide_congr' {p q : Prop} [Decidable p] [Decidable q] (h : p ↔ q) :
    decide p = decide q := by
  by_cases hp : p
  · rw [decide_eq_true hp, decide_eq_true (h.mp hp)]
  · rw [decide_eq_false hp, decide_eq_false (fun hq => hp (h.mpr hq))]

theorem band_left : ∀ {a b : Bool}, (a && b) = true → a = true
  | true, _, _ => rfl
  | false, _, h => h

theorem band_right : ∀ {a b : Bool}, (a && b) = true → b = true
  | true, _, h => h
  | false, _, h => Bool.noConfusion h

theorem bool_false_of_ne_true : ∀ {b : Bool}, ¬ b = true → b = false
  | false, _ => rfl
  | true, h => absurd rfl h

theorem bool_true_of_ne_false : ∀ {b : Bool}, ¬ b = false → b = true
  | true, _ => rfl
  | false, h => absurd rfl h

theorem succ_ne_zero' (a : Nat) : a + 1 ≠ 0 := fun h => Nat.noConfusion h

/-- The `n`-th element of a list, if any. -/
def nth {α : Type} : List α → Nat → Option α
  | [], _ => none
  | x :: _, 0 => some x
  | _ :: l, n + 1 => nth l n

theorem nth_lt {α : Type} : ∀ (l : List α) (n : Nat) (x : α), nth l n = some x → n < l.length
  | [], _, _, h => by cases h
  | _ :: _, 0, _, _ => Nat.succ_pos _
  | _ :: l, n + 1, x, h => Nat.succ_lt_succ (nth_lt l n x h)

/-- Membership of a manifest index in the list of items already read. -/
def memB (k : Nat) : List Nat → Bool
  | [] => false
  | x :: xs => decide (k = x) || memB k xs

theorem memB_head (k : Nat) (xs : List Nat) : memB k (k :: xs) = true := by
  show (decide (k = k) || memB k xs) = true
  rw [decide_eq_true rfl]
  rfl

theorem memB_cons_of_ne {k x : Nat} (xs : List Nat) (h : k ≠ x) :
    memB k (x :: xs) = memB k xs := by
  show (decide (k = x) || memB k xs) = memB k xs
  rw [decide_eq_false h]
  rfl

/-! ## OPF element roles (EPUB 3.3 package document)

Element names are coded `1` package, `2` manifest, `3` spine, `4` item, `5` itemref, `0` any
other local name; `opfNs` says the element is in the OPF namespace. Roles are `0` document
(the context of the root element), `1` package, `2` manifest, `3` spine, `4` manifest item,
`5` spine itemref, `6` ignored (and everything inside an ignored element is ignored). -/

def childRole (parent name : Nat) (opfNs : Bool) : Nat :=
  if opfNs = false then 6
  else if parent = 0 ∧ name = 1 then 1
  else if parent = 1 ∧ name = 2 then 2
  else if parent = 1 ∧ name = 3 then 3
  else if parent = 2 ∧ name = 4 then 4
  else if parent = 3 ∧ name = 5 then 5
  else 6

/-- The role of the last element of a path of `(name, opfNs)` pairs from the root down. -/
def roleFrom (r : Nat) : List (Nat × Bool) → Nat
  | [] => r
  | e :: rest => roleFrom (childRole r e.1 e.2) rest

def roleOf (path : List (Nat × Bool)) : Nat := roleFrom 0 path

/-- Every role `childRole` returns, with the only inputs that produce it. -/
theorem childRole_cases (p n : Nat) (b : Bool) :
    childRole p n b = 6 ∨
    (b = true ∧ p = 0 ∧ n = 1 ∧ childRole p n b = 1) ∨
    (b = true ∧ p = 1 ∧ n = 2 ∧ childRole p n b = 2) ∨
    (b = true ∧ p = 1 ∧ n = 3 ∧ childRole p n b = 3) ∨
    (b = true ∧ p = 2 ∧ n = 4 ∧ childRole p n b = 4) ∨
    (b = true ∧ p = 3 ∧ n = 5 ∧ childRole p n b = 5) := by
  unfold childRole
  cases b with
  | false => exact Or.inl (if_pos rfl)
  | true =>
    have hb : ¬ (true = false) := Bool.noConfusion
    rw [if_neg hb]
    by_cases h1 : p = 0 ∧ n = 1
    · rw [if_pos h1]; exact Or.inr (Or.inl ⟨rfl, h1.1, h1.2, rfl⟩)
    rw [if_neg h1]
    by_cases h2 : p = 1 ∧ n = 2
    · rw [if_pos h2]; exact Or.inr (Or.inr (Or.inl ⟨rfl, h2.1, h2.2, rfl⟩))
    rw [if_neg h2]
    by_cases h3 : p = 1 ∧ n = 3
    · rw [if_pos h3]; exact Or.inr (Or.inr (Or.inr (Or.inl ⟨rfl, h3.1, h3.2, rfl⟩)))
    rw [if_neg h3]
    by_cases h4 : p = 2 ∧ n = 4
    · rw [if_pos h4]; exact Or.inr (Or.inr (Or.inr (Or.inr (Or.inl ⟨rfl, h4.1, h4.2, rfl⟩))))
    rw [if_neg h4]
    by_cases h5 : p = 3 ∧ n = 5
    · rw [if_pos h5]
      exact Or.inr (Or.inr (Or.inr (Or.inr (Or.inr ⟨rfl, h5.1, h5.2, rfl⟩))))
    rw [if_neg h5]
    exact Or.inl rfl

/-- A manifest item is only an OPF `item` directly inside the manifest. -/
theorem childRole_item {p n : Nat} {b : Bool} (h : childRole p n b = 4) :
    b = true ∧ p = 2 ∧ n = 4 := by
  rcases childRole_cases p n b with e | ⟨_, _, _, e⟩ | ⟨_, _, _, e⟩ | ⟨_, _, _, e⟩ |
      ⟨hb, hp, hn, _⟩ | ⟨_, _, _, e⟩
  · exact absurd (h.symm.trans e) (by decide)
  · exact absurd (h.symm.trans e) (by decide)
  · exact absurd (h.symm.trans e) (by decide)
  · exact absurd (h.symm.trans e) (by decide)
  · exact ⟨hb, hp, hn⟩
  · exact absurd (h.symm.trans e) (by decide)

/-- A spine reference is only an OPF `itemref` directly inside the spine. -/
theorem childRole_itemref {p n : Nat} {b : Bool} (h : childRole p n b = 5) :
    b = true ∧ p = 3 ∧ n = 5 := by
  rcases childRole_cases p n b with e | ⟨_, _, _, e⟩ | ⟨_, _, _, e⟩ | ⟨_, _, _, e⟩ |
      ⟨_, _, _, e⟩ | ⟨hb, hp, hn, _⟩
  · exact absurd (h.symm.trans e) (by decide)
  · exact absurd (h.symm.trans e) (by decide)
  · exact absurd (h.symm.trans e) (by decide)
  · exact absurd (h.symm.trans e) (by decide)
  · exact absurd (h.symm.trans e) (by decide)
  · exact ⟨hb, hp, hn⟩

/-- Nothing below an element with role `p ≥ 4` (an item, an itemref, or an ignored element)
has a role other than ignored. -/
theorem childRole_leaf {p : Nat} (hp : 4 ≤ p) (n : Nat) (b : Bool) : childRole p n b = 6 := by
  rcases childRole_cases p n b with e | ⟨_, h, _, _⟩ | ⟨_, h, _, _⟩ | ⟨_, h, _, _⟩ |
      ⟨_, h, _, _⟩ | ⟨_, h, _, _⟩
  · exact e
  all_goals (subst h; exact absurd hp (by decide))

theorem roleFrom_ignored : ∀ (path : List (Nat × Bool)), roleFrom 6 path = 6
  | [] => rfl
  | e :: rest => by
      show roleFrom (childRole 6 e.1 e.2) rest = 6
      rw [childRole_leaf (by decide) e.1 e.2]
      exact roleFrom_ignored rest

theorem roleFrom_leaf {p : Nat} (hp : 4 ≤ p) :
    ∀ (e : Nat × Bool) (rest : List (Nat × Bool)), roleFrom p (e :: rest) = 6 := by
  intro e rest
  show roleFrom (childRole p e.1 e.2) rest = 6
  rw [childRole_leaf hp e.1 e.2]
  exact roleFrom_ignored rest

/-- From the spine element, no descendant is a manifest item. -/
theorem roleFrom_spine_not_item : ∀ (path : List (Nat × Bool)), roleFrom 3 path ≠ 4
  | [], h => absurd h (by decide)
  | e :: rest, h => by
      have h' : roleFrom (childRole 3 e.1 e.2) rest = 4 := h
      rcases childRole_cases 3 e.1 e.2 with c | ⟨_, c, _, _⟩ | ⟨_, c, _, _⟩ | ⟨_, c, _, _⟩ |
          ⟨_, c, _, _⟩ | ⟨_, _, _, c⟩
      · rw [c, roleFrom_ignored] at h'; exact absurd h' (by decide)
      · exact absurd c (by decide)
      · exact absurd c (by decide)
      · exact absurd c (by decide)
      · exact absurd c (by decide)
      · rw [c] at h'
        cases rest with
        | nil => exact absurd h' (by decide)
        | cons e' rest' => rw [roleFrom_leaf (by decide)] at h'; exact absurd h' (by decide)

/-- From the manifest element, no descendant is a spine itemref. -/
theorem roleFrom_manifest_not_itemref : ∀ (path : List (Nat × Bool)), roleFrom 2 path ≠ 5
  | [], h => absurd h (by decide)
  | e :: rest, h => by
      have h' : roleFrom (childRole 2 e.1 e.2) rest = 5 := h
      rcases childRole_cases 2 e.1 e.2 with c | ⟨_, c, _, _⟩ | ⟨_, c, _, _⟩ | ⟨_, c, _, _⟩ |
          ⟨_, _, _, c⟩ | ⟨_, c, _, _⟩
      · rw [c, roleFrom_ignored] at h'; exact absurd h' (by decide)
      · exact absurd c (by decide)
      · exact absurd c (by decide)
      · exact absurd c (by decide)
      · rw [c] at h'
        cases rest with
        | nil => exact absurd h' (by decide)
        | cons e' rest' => rw [roleFrom_leaf (by decide)] at h'; exact absurd h' (by decide)
      · exact absurd c (by decide)

/-- After the package element, the only path to a manifest item is `manifest/item`. -/
theorem roleFrom_package_item : ∀ (path : List (Nat × Bool)), roleFrom 1 path = 4 →
    path = [(2, true), (4, true)]
  | [], h => absurd h (by decide)
  | e :: rest, h => by
      have h' : roleFrom (childRole 1 e.1 e.2) rest = 4 := h
      rcases childRole_cases 1 e.1 e.2 with c | ⟨_, c, _, _⟩ | ⟨hb, _, hn, c⟩ |
          ⟨_, _, _, c⟩ | ⟨_, c, _, _⟩ | ⟨_, c, _, _⟩
      · rw [c, roleFrom_ignored] at h'; exact absurd h' (by decide)
      · exact absurd c (by decide)
      · rw [c] at h'
        cases rest with
        | nil => exact absurd h' (by decide)
        | cons e' rest' =>
          have h'' : roleFrom (childRole 2 e'.1 e'.2) rest' = 4 := h'
          rcases childRole_cases 2 e'.1 e'.2 with c' | ⟨_, c', _, _⟩ | ⟨_, c', _, _⟩ |
              ⟨_, c', _, _⟩ | ⟨hb', _, hn', c'⟩ | ⟨_, c', _, _⟩
          · rw [c', roleFrom_ignored] at h''; exact absurd h'' (by decide)
          · exact absurd c' (by decide)
          · exact absurd c' (by decide)
          · exact absurd c' (by decide)
          · rw [c'] at h''
            cases rest' with
            | nil =>
              cases e with
              | mk a b =>
                cases e' with
                | mk a' b' =>
                  dsimp only at hb hn hb' hn'
                  subst hb hn hb' hn'
                  rfl
            | cons _ _ => rw [roleFrom_leaf (by decide)] at h''; exact absurd h'' (by decide)
          · exact absurd c' (by decide)
      · rw [c] at h'; exact absurd h' (roleFrom_spine_not_item rest)
      · exact absurd c (by decide)
      · exact absurd c (by decide)

/-- The only element path whose role is a manifest item is `package/manifest/item`, each in
the OPF namespace: an `item` in the metadata, in a collection, nested deeper, or in another
namespace is never a manifest entry. -/
theorem roleOf_item_iff (path : List (Nat × Bool)) :
    roleOf path = 4 ↔ path = [(1, true), (2, true), (4, true)] := by
  constructor
  · intro h
    cases path with
    | nil => exact absurd h (by decide)
    | cons e rest =>
      have h' : roleFrom (childRole 0 e.1 e.2) rest = 4 := h
      rcases childRole_cases 0 e.1 e.2 with c | ⟨hb, _, hn, c⟩ | ⟨_, c, _, _⟩ |
          ⟨_, c, _, _⟩ | ⟨_, c, _, _⟩ | ⟨_, c, _, _⟩
      · rw [c, roleFrom_ignored] at h'; exact absurd h' (by decide)
      · rw [c] at h'
        have hr := roleFrom_package_item rest h'
        cases e with
        | mk a b =>
          dsimp only at hb hn
          subst hb hn hr
          rfl
      · exact absurd c (by decide)
      · exact absurd c (by decide)
      · exact absurd c (by decide)
      · exact absurd c (by decide)
  · intro h
    subst h
    rfl

/-- The only element path whose role is a spine itemref is `package/spine/itemref`. -/
theorem roleOf_itemref_iff (path : List (Nat × Bool)) :
    roleOf path = 5 ↔ path = [(1, true), (3, true), (5, true)] := by
  constructor
  · intro h
    cases path with
    | nil => exact absurd h (by decide)
    | cons e rest =>
      have h' : roleFrom (childRole 0 e.1 e.2) rest = 5 := h
      rcases childRole_cases 0 e.1 e.2 with c | ⟨hb, _, hn, c⟩ | ⟨_, c, _, _⟩ |
          ⟨_, c, _, _⟩ | ⟨_, c, _, _⟩ | ⟨_, c, _, _⟩
      · rw [c, roleFrom_ignored] at h'; exact absurd h' (by decide)
      · rw [c] at h'
        cases rest with
        | nil => exact absurd h' (by decide)
        | cons e' rest' =>
          have h'' : roleFrom (childRole 1 e'.1 e'.2) rest' = 5 := h'
          rcases childRole_cases 1 e'.1 e'.2 with c' | ⟨_, c', _, _⟩ | ⟨_, _, _, c'⟩ |
              ⟨hb', _, hn', c'⟩ | ⟨_, c', _, _⟩ | ⟨_, c', _, _⟩
          · rw [c', roleFrom_ignored] at h''; exact absurd h'' (by decide)
          · exact absurd c' (by decide)
          · rw [c'] at h''; exact absurd h'' (roleFrom_manifest_not_itemref rest')
          · rw [c'] at h''
            cases rest' with
            | nil => exact absurd h'' (by decide)
            | cons e'' rest'' =>
              have h3 : roleFrom (childRole 3 e''.1 e''.2) rest'' = 5 := h''
              rcases childRole_cases 3 e''.1 e''.2 with c'' | ⟨_, c'', _, _⟩ |
                  ⟨_, c'', _, _⟩ | ⟨_, c'', _, _⟩ | ⟨_, c'', _, _⟩ | ⟨hb'', _, hn'', c''⟩
              · rw [c'', roleFrom_ignored] at h3; exact absurd h3 (by decide)
              · exact absurd c'' (by decide)
              · exact absurd c'' (by decide)
              · exact absurd c'' (by decide)
              · exact absurd c'' (by decide)
              · rw [c''] at h3
                cases rest'' with
                | nil =>
                  cases e with
                  | mk a b =>
                    cases e' with
                    | mk a' b' =>
                      cases e'' with
                      | mk a'' b'' =>
                        dsimp only at hb hn hb' hn' hb'' hn''
                        subst hb hn hb' hn' hb'' hn''
                        rfl
                | cons _ _ => rw [roleFrom_leaf (by decide)] at h3; exact absurd h3 (by decide)
          · exact absurd c' (by decide)
          · exact absurd c' (by decide)
      · exact absurd c (by decide)
      · exact absurd c (by decide)
      · exact absurd c (by decide)
      · exact absurd c (by decide)
  · intro h
    subst h
    rfl

/-! ## Manifest items (EPUB 3.3 `item`; XML 1.0 3.3.1)

An `item` enters the manifest only with both required attributes and a non-empty id. -/

def itemAccept (hasId hasHref : Bool) (idLen : Nat) : Bool :=
  hasId && hasHref && decide (0 < idLen)

theorem itemAccept_spec {hasId hasHref : Bool} {idLen : Nat}
    (h : itemAccept hasId hasHref idLen = true) :
    hasId = true ∧ hasHref = true ∧ 0 < idLen :=
  ⟨band_left (band_left h), band_right (band_left h), of_decide_eq_true (band_right h)⟩

/-- A manifest entry: its id (an interned id string; only equality matters), its `href`
bytes and its media type (`1` XHTML, `2` SVG, anything else foreign). -/
structure Item where
  id : Nat
  href : List Nat
  media : Nat

/-- EPUB content documents (EPUB 3.3 3.2): XHTML and SVG. -/
def isContentDoc (media : Nat) : Bool := decide (media = 1) || decide (media = 2)

/-! ## The id lookup, declaratively

`countId i m` is how many manifest entries have id `i`, `firstIndex i m` the position of
the first. -/

def countId (i : Nat) : List Item → Nat
  | [] => 0
  | it :: m => if it.id = i then countId i m + 1 else countId i m

def firstIndex (i : Nat) : List Item → Option Nat
  | [] => none
  | it :: m =>
    if it.id = i then some 0
    else match firstIndex i m with
      | none => none
      | some k => some (k + 1)

theorem countId_pos_cons {i : Nat} {it : Item} (m : List Item) (h : it.id = i) :
    countId i (it :: m) = countId i m + 1 := if_pos h

theorem countId_neg_cons {i : Nat} {it : Item} (m : List Item) (h : ¬ it.id = i) :
    countId i (it :: m) = countId i m := if_neg h

theorem countId_le_cons (i : Nat) (it : Item) (m : List Item) :
    countId i m ≤ countId i (it :: m) := by
  by_cases h : it.id = i
  · rw [countId_pos_cons m h]; exact Nat.le_succ _
  · rw [countId_neg_cons m h]; exact Nat.le_refl _

theorem firstIndex_sound {i : Nat} :
    ∀ (m : List Item) (k : Nat), firstIndex i m = some k → ∃ it, nth m k = some it ∧ it.id = i
  | [], _, h => by cases h
  | it :: m, k, h => by
      by_cases hid : it.id = i
      · have h' : some 0 = some k := by
          have e : firstIndex i (it :: m) = some 0 := if_pos hid
          rw [← e]; exact h
        cases h'
        exact ⟨it, rfl, hid⟩
      · have e : firstIndex i (it :: m) =
            (match firstIndex i m with | none => none | some k => some (k + 1)) := if_neg hid
        rw [e] at h
        cases hf : firstIndex i m with
        | none => rw [hf] at h; cases h
        | some k' =>
          rw [hf] at h
          cases h
          exact firstIndex_sound m k' hf

theorem firstIndex_least {i : Nat} :
    ∀ (m : List Item) (k : Nat), firstIndex i m = some k →
      ∀ (j : Nat) (it : Item), j < k → nth m j = some it → it.id ≠ i
  | [], _, h, _, _, _, _ => by cases h
  | it :: m, k, h, j, it', hj, hn => by
      by_cases hid : it.id = i
      · have e : firstIndex i (it :: m) = some 0 := if_pos hid
        rw [e] at h
        cases h
        exact absurd hj (Nat.not_lt_zero _)
      · have e : firstIndex i (it :: m) =
            (match firstIndex i m with | none => none | some k => some (k + 1)) := if_neg hid
        rw [e] at h
        cases hf : firstIndex i m with
        | none => rw [hf] at h; cases h
        | some k' =>
          rw [hf] at h
          cases h
          cases j with
          | zero =>
            have : it = it' := by cases hn; rfl
            subst this
            exact hid
          | succ j => exact firstIndex_least m k' hf j it' (Nat.lt_of_succ_lt_succ hj) hn

theorem firstIndex_none {i : Nat} :
    ∀ (m : List Item), firstIndex i m = none →
      ∀ (j : Nat) (it : Item), nth m j = some it → it.id ≠ i
  | [], _, _, _, hn => by cases hn
  | it :: m, h, j, it', hn => by
      by_cases hid : it.id = i
      · have e : firstIndex i (it :: m) = some 0 := if_pos hid
        rw [e] at h
        cases h
      · have e : firstIndex i (it :: m) =
            (match firstIndex i m with | none => none | some k => some (k + 1)) := if_neg hid
        rw [e] at h
        cases hf : firstIndex i m with
        | some k' => rw [hf] at h; cases h
        | none =>
          cases j with
          | zero =>
            have : it = it' := by cases hn; rfl
            subst this
            exact hid
          | succ j => exact firstIndex_none m hf j it' hn

theorem firstIndex_of_mem {i : Nat} (m : List Item) {j : Nat} {it : Item}
    (hn : nth m j = some it) (hid : it.id = i) : ∃ k, firstIndex i m = some k ∧ k ≤ j := by
  cases hf : firstIndex i m with
  | none => exact absurd hid (firstIndex_none m hf j it hn)
  | some k =>
    refine ⟨k, rfl, ?_⟩
    cases Nat.lt_or_ge j k with
    | inl hjk => exact absurd hid (firstIndex_least m k hf j it hjk hn)
    | inr hkj => exact hkj

theorem countId_pos_of_mem {i : Nat} :
    ∀ (m : List Item) (j : Nat) (it : Item), nth m j = some it → it.id = i → 0 < countId i m
  | [], _, _, hn, _ => by cases hn
  | it :: m, 0, it', hn, hid => by
      have : it = it' := by cases hn; rfl
      subst this
      rw [countId_pos_cons m hid]
      exact Nat.succ_pos _
  | it :: m, j + 1, it', hn, hid =>
      Nat.lt_of_lt_of_le (countId_pos_of_mem m j it' hn hid) (countId_le_cons i it m)

theorem countId_pos_witness {i : Nat} :
    ∀ (m : List Item), 0 < countId i m → ∃ j it, nth m j = some it ∧ it.id = i
  | [], h => absurd h (Nat.lt_irrefl 0)
  | it :: m, h => by
      by_cases hid : it.id = i
      · exact ⟨0, it, rfl, hid⟩
      · rw [countId_neg_cons m hid] at h
        have ⟨j, it', hn, hid'⟩ := countId_pos_witness m h
        exact ⟨j + 1, it', hn, hid'⟩

theorem countId_two_of_two {i : Nat} :
    ∀ (m : List Item) (j₁ j₂ : Nat) (it₁ it₂ : Item), j₁ < j₂ →
      nth m j₁ = some it₁ → nth m j₂ = some it₂ → it₁.id = i → it₂.id = i → 2 ≤ countId i m
  | [], _, _, _, _, _, h, _, _, _ => by cases h
  | _ :: _, _, 0, _, _, hj, _, _, _, _ => absurd hj (Nat.not_lt_zero _)
  | it :: m, 0, j₂ + 1, it₁, it₂, _, h₁, h₂, e₁, e₂ => by
      have : it = it₁ := by cases h₁; rfl
      subst this
      rw [countId_pos_cons m e₁]
      exact Nat.succ_le_succ (countId_pos_of_mem m j₂ it₂ h₂ e₂)
  | it :: m, j₁ + 1, j₂ + 1, it₁, it₂, hj, h₁, h₂, e₁, e₂ =>
      Nat.le_trans (countId_two_of_two m j₁ j₂ it₁ it₂ (Nat.lt_of_succ_lt_succ hj) h₁ h₂ e₁ e₂)
        (countId_le_cons i it m)

theorem countId_two_witness {i : Nat} :
    ∀ (m : List Item), 2 ≤ countId i m →
      ∃ j₁ j₂ it₁ it₂, j₁ < j₂ ∧ nth m j₁ = some it₁ ∧ nth m j₂ = some it₂ ∧
        it₁.id = i ∧ it₂.id = i
  | [], h => absurd h (Nat.not_succ_le_zero 1)
  | it :: m, h => by
      by_cases hid : it.id = i
      · rw [countId_pos_cons m hid] at h
        have ⟨j, it', hn, hid'⟩ := countId_pos_witness m (Nat.le_of_succ_le_succ h)
        exact ⟨0, j + 1, it, it', Nat.succ_pos j, rfl, hn, hid, hid'⟩
      · rw [countId_neg_cons m hid] at h
        have ⟨j₁, j₂, it₁, it₂, hj, h₁, h₂, e₁, e₂⟩ := countId_two_witness m h
        exact ⟨j₁ + 1, j₂ + 1, it₁, it₂, Nat.succ_lt_succ hj, h₁, h₂, e₁, e₂⟩

theorem countId_one_unique {i : Nat} (m : List Item) (hc : countId i m = 1)
    {j₁ j₂ : Nat} {it₁ it₂ : Item} (h₁ : nth m j₁ = some it₁) (h₂ : nth m j₂ = some it₂)
    (e₁ : it₁.id = i) (e₂ : it₂.id = i) : j₁ = j₂ := by
  cases Nat.lt_or_ge j₁ j₂ with
  | inl hlt =>
    have := countId_two_of_two m j₁ j₂ it₁ it₂ hlt h₁ h₂ e₁ e₂
    rw [hc] at this
    exact absurd this (by decide)
  | inr hge =>
    cases Nat.lt_or_ge j₂ j₁ with
    | inl hlt =>
      have := countId_two_of_two m j₂ j₁ it₂ it₁ hlt h₂ h₁ e₂ e₁
      rw [hc] at this
      exact absurd this (by decide)
    | inr hle => exact Nat.le_antisymm hle hge

/-! ## Resolving one id (EPUB 3.3 `itemref@idref`, XML 1.0 3.3.1) -/

inductive Lookup where
  | item (k : Nat)
  | missing
  | duplicate
  deriving DecidableEq

/-- The meaning of a spine `idref` against the manifest. -/
def resolveId (m : List Item) (i : Nat) : Lookup :=
  match firstIndex i m with
  | none => .missing
  | some k => if countId i m = 1 then .item k else .duplicate

theorem resolveId_none {m : List Item} {i : Nat} (hf : firstIndex i m = none) :
    resolveId m i = .missing := by
  unfold resolveId; rw [hf]

theorem resolveId_some {m : List Item} {i k : Nat} (hf : firstIndex i m = some k) :
    resolveId m i = if countId i m = 1 then .item k else .duplicate := by
  unfold resolveId; rw [hf]

/-- Never a wrong item: a resolved id names a manifest entry that has that id. -/
theorem resolveId_item_sound {m : List Item} {i k : Nat} (h : resolveId m i = .item k) :
    ∃ it, nth m k = some it ∧ it.id = i := by
  cases hf : firstIndex i m with
  | none => rw [resolveId_none hf] at h; cases h
  | some k' =>
    rw [resolveId_some hf] at h
    by_cases hc : countId i m = 1
    · rw [if_pos hc] at h
      cases h
      exact firstIndex_sound m _ hf
    · rw [if_neg hc] at h; cases h

theorem resolveId_item_count {m : List Item} {i k : Nat} (h : resolveId m i = .item k) :
    countId i m = 1 := by
  cases hf : firstIndex i m with
  | none => rw [resolveId_none hf] at h; cases h
  | some k' =>
    rw [resolveId_some hf] at h
    by_cases hc : countId i m = 1
    · exact hc
    · rw [if_neg hc] at h; cases h

/-- Exactly that entry: no other manifest entry has the id. -/
theorem resolveId_item_unique {m : List Item} {i k : Nat} (h : resolveId m i = .item k)
    {j : Nat} {it : Item} (hn : nth m j = some it) (hid : it.id = i) : j = k := by
  have ⟨it', hk, hid'⟩ := resolveId_item_sound h
  exact countId_one_unique m (resolveId_item_count h) hn hk hid hid'

/-- Missing ids are reported, and only they. -/
theorem resolveId_missing_iff (m : List Item) (i : Nat) :
    resolveId m i = .missing ↔ ∀ (j : Nat) (it : Item), nth m j = some it → it.id ≠ i := by
  constructor
  · intro h
    cases hf : firstIndex i m with
    | none => exact firstIndex_none m hf
    | some k =>
      rw [resolveId_some hf] at h
      by_cases hc : countId i m = 1
      · rw [if_pos hc] at h; cases h
      · rw [if_neg hc] at h; cases h
  · intro h
    cases hf : firstIndex i m with
    | none => exact resolveId_none hf
    | some k =>
      have ⟨it, hn, hid⟩ := firstIndex_sound m k hf
      exact absurd hid (h k it hn)

/-- A duplicated id is reported, and only a duplicated id. -/
theorem resolveId_duplicate_iff (m : List Item) (i : Nat) :
    resolveId m i = .duplicate ↔
      ∃ j₁ j₂ it₁ it₂, j₁ < j₂ ∧ nth m j₁ = some it₁ ∧ nth m j₂ = some it₂ ∧
        it₁.id = i ∧ it₂.id = i := by
  constructor
  · intro h
    cases hf : firstIndex i m with
    | none => rw [resolveId_none hf] at h; cases h
    | some k =>
      rw [resolveId_some hf] at h
      have ⟨it, hn, hid⟩ := firstIndex_sound m k hf
      have hpos := countId_pos_of_mem m k it hn hid
      by_cases hc : countId i m = 1
      · rw [if_pos hc] at h; cases h
      · apply countId_two_witness m
        revert hc hpos
        generalize countId i m = c
        intro hpos hc
        cases c with
        | zero => exact absurd hpos (Nat.lt_irrefl 0)
        | succ c =>
          cases c with
          | zero => exact absurd rfl hc
          | succ c => exact Nat.succ_le_succ (Nat.succ_le_succ (Nat.zero_le c))
  · intro ⟨j₁, j₂, it₁, it₂, hj, h₁, h₂, e₁, e₂⟩
    have h2 := countId_two_of_two m j₁ j₂ it₁ it₂ hj h₁ h₂ e₁ e₂
    have ⟨k, hk, _⟩ := firstIndex_of_mem m h₁ e₁
    rw [resolveId_some hk]
    have hc : ¬ countId i m = 1 := fun e => absurd (e ▸ h2) (by decide)
    rw [if_neg hc]

/-- Complete: an id that exactly one entry has resolves to that entry. -/
theorem resolveId_item_complete {m : List Item} {i k : Nat} {it : Item}
    (hn : nth m k = some it) (hid : it.id = i)
    (huniq : ∀ (j : Nat) (it' : Item), nth m j = some it' → it'.id = i → j = k) :
    resolveId m i = .item k := by
  have hpos := countId_pos_of_mem m k it hn hid
  have hc : countId i m = 1 := by
    revert hpos
    generalize hcg : countId i m = c
    intro hpos
    cases c with
    | zero => exact absurd hpos (Nat.lt_irrefl 0)
    | succ c =>
      cases c with
      | zero => rfl
      | succ c =>
        have h2 : 2 ≤ countId i m := by
          rw [hcg]; exact Nat.succ_le_succ (Nat.succ_le_succ (Nat.zero_le c))
        have ⟨j₁, j₂, it₁, it₂, hj, h₁, h₂, e₁, e₂⟩ := countId_two_witness m h2
        have a := huniq j₁ it₁ h₁ e₁
        have b := huniq j₂ it₂ h₂ e₂
        rw [a, b] at hj
        exact absurd hj (Nat.lt_irrefl k)
  have ⟨k', hk', _⟩ := firstIndex_of_mem m hn hid
  have ⟨it', hn', hid'⟩ := firstIndex_sound m k' hk'
  have hkk : k' = k := huniq k' it' hn' hid'
  rw [resolveId_some hk', if_pos hc, hkk]

/-! ## The table-backed index (apeiron `Table`/`HMap` shape)

The production index is a table from id to `(first index, duplicated)`, built by one insert
per manifest entry in document order. `indexStep` is the value written for entry `k` given
what the table held for its id. -/

def indexStep (prev : Option (Nat × Bool)) (k : Nat) : Nat × Bool :=
  match prev with
  | none => (k, false)
  | some p => (p.1, true)

/-- The field function of `manifest_index_step`. -/
def indexFields (present : Bool) (prevIndex k : Nat) : Nat × Bool :=
  if present = true then (prevIndex, true) else (k, false)

theorem indexStep_fields (prev : Option (Nat × Bool)) (k : Nat) :
    indexStep prev k = indexFields prev.isSome (prev.getD (0, false)).1 k := by
  cases prev with
  | none => rfl
  | some p => rfl

abbrev Index := Nat → Option (Nat × Bool)

def emptyIndex : Index := fun _ => none

def insertItem (t : Index) (key k : Nat) : Index :=
  fun i => if i = key then some (indexStep (t key) k) else t i

def buildFrom : Index → Nat → List Item → Index
  | t, _, [] => t
  | t, k, it :: m => buildFrom (insertItem t it.id k) (k + 1) m

def buildIndex (m : List Item) : Index := buildFrom emptyIndex 0 m

def lookupIndex (t : Index) (i : Nat) : Lookup :=
  match t i with
  | none => .missing
  | some p => if p.2 = true then .duplicate else .item p.1

/-- What the table holds for `i` after inserting the entries `m` from position `k` on. -/
def mergeTail (prev : Option (Nat × Bool)) (k : Nat) (m : List Item) (i : Nat) :
    Option (Nat × Bool) :=
  match prev with
  | some p => some (p.1, p.2 || decide (0 < countId i m))
  | none =>
    match firstIndex i m with
    | none => none
    | some j => some (k + j, decide (1 < countId i m))

theorem buildFrom_eq (i : Nat) :
    ∀ (m : List Item) (t : Index) (k : Nat), buildFrom t k m i = mergeTail (t i) k m i
  | [], t, k => by
      show t i = mergeTail (t i) k [] i
      generalize t i = o
      cases o with
      | none => rfl
      | some p =>
        cases p with
        | mk a b => cases b <;> rfl
  | it :: m, t, k => by
      show buildFrom (insertItem t it.id k) (k + 1) m i = mergeTail (t i) k (it :: m) i
      rw [buildFrom_eq i m (insertItem t it.id k) (k + 1)]
      show mergeTail (if i = it.id then some (indexStep (t it.id) k) else t i) (k + 1) m i =
        mergeTail (t i) k (it :: m) i
      by_cases hid : it.id = i
      · rw [if_pos hid.symm, hid]
        have hf : firstIndex i (it :: m) = some 0 := if_pos hid
        cases ht : t i with
        | none =>
          show some (k, false || decide (0 < countId i m)) =
            (match firstIndex i (it :: m) with
              | none => none
              | some j => some (k + j, decide (1 < countId i (it :: m))))
          rw [hf, countId_pos_cons m hid]
          show some (k, decide (0 < countId i m)) = some (k, decide (1 < countId i m + 1))
          rw [decide_congr' ⟨Nat.succ_lt_succ, Nat.lt_of_succ_lt_succ⟩]
        | some p =>
          show some (p.1, true || decide (0 < countId i m)) =
            some (p.1, p.2 || decide (0 < countId i (it :: m)))
          rw [countId_pos_cons m hid, decide_eq_true (Nat.succ_pos _)]
          cases p.2 <;> rfl
      · have hne : ¬ i = it.id := fun e => hid e.symm
        rw [if_neg hne]
        have hf : firstIndex i (it :: m) =
            (match firstIndex i m with | none => none | some k => some (k + 1)) := if_neg hid
        cases ht : t i with
        | none =>
          show (match firstIndex i m with
              | none => none
              | some j => some (k + 1 + j, decide (1 < countId i m))) =
            (match firstIndex i (it :: m) with
              | none => none
              | some j => some (k + j, decide (1 < countId i (it :: m))))
          rw [hf, countId_neg_cons m hid]
          cases firstIndex i m with
          | none => rfl
          | some j =>
            show some (k + 1 + j, _) = some (k + (j + 1), _)
            rw [Nat.add_assoc, Nat.add_comm 1 j]
        | some p =>
          show some (p.1, p.2 || decide (0 < countId i m)) =
            some (p.1, p.2 || decide (0 < countId i (it :: m)))
          rw [countId_neg_cons m hid]

/-- The table built by inserting each manifest entry in order answers every id exactly as
the declarative `resolveId`: the table-backed index refines the specification. -/
theorem lookupIndex_build (m : List Item) (i : Nat) :
    lookupIndex (buildIndex m) i = resolveId m i := by
  unfold lookupIndex buildIndex resolveId
  rw [buildFrom_eq i m emptyIndex 0]
  show (match (match firstIndex i m with
      | none => none
      | some j => some (0 + j, decide (1 < countId i m))) with
    | none => Lookup.missing
    | some p => if p.2 = true then Lookup.duplicate else Lookup.item p.1) =
    (match firstIndex i m with
      | none => Lookup.missing
      | some k => if countId i m = 1 then Lookup.item k else Lookup.duplicate)
  cases hf : firstIndex i m with
  | none => rfl
  | some j =>
    have ⟨it, hn, hid⟩ := firstIndex_sound m j hf
    have hpos := countId_pos_of_mem m j it hn hid
    show (if decide (1 < countId i m) = true then Lookup.duplicate else Lookup.item (0 + j)) =
      (if countId i m = 1 then Lookup.item j else Lookup.duplicate)
    rw [Nat.zero_add]
    revert hpos
    generalize countId i m = c
    intro hpos
    cases c with
    | zero => exact absurd hpos (Nat.lt_irrefl 0)
    | succ c =>
      cases c with
      | zero => rfl
      | succ c =>
        rw [if_pos (decide_eq_true (Nat.succ_lt_succ (Nat.succ_pos c)))]
        rw [if_neg (fun e => absurd (Nat.succ.inj e) (succ_ne_zero' c))]

/-! ## The spine (EPUB 3.3 spine; epubcheck OPF-034)

Each `itemref` yields exactly one step, in spine order. -/

inductive Step where
  | read (k : Nat)
  | missing
  | duplicate
  | foreign (k : Nat)
  | repeated (k : Nat)
  deriving DecidableEq

/-- Whether manifest entry `k` is an EPUB content document. -/
def contentAt (m : List Item) (k : Nat) : Bool :=
  match nth m k with
  | some it => isContentDoc it.media
  | none => false

def spineStep (m : List Item) (seen : List Nat) (i : Nat) : Step :=
  match resolveId m i with
  | .missing => .missing
  | .duplicate => .duplicate
  | .item k =>
    if contentAt m k = true then
      if memB k seen = true then .repeated k else .read k
    else .foreign k

theorem spineStep_item {m : List Item} {seen : List Nat} {i k : Nat}
    (h : resolveId m i = .item k) : spineStep m seen i =
      if contentAt m k = true then (if memB k seen = true then .repeated k else .read k)
      else .foreign k := by
  unfold spineStep; rw [h]

def stepSeen : Step → List Nat → List Nat
  | .read k, seen => k :: seen
  | .missing, seen => seen
  | .duplicate, seen => seen
  | .foreign _, seen => seen
  | .repeated _, seen => seen

def spineGo (m : List Item) : List Nat → List Nat → List Step
  | _, [] => []
  | seen, i :: s => spineStep m seen i :: spineGo m (stepSeen (spineStep m seen i) seen) s

def resolveSpine (m : List Item) (spine : List Nat) : List Step := spineGo m [] spine

theorem spineGo_length (m : List Item) :
    ∀ (s seen : List Nat), (spineGo m seen s).length = s.length
  | [], _ => rfl
  | _ :: s, _ => congrArg (· + 1) (spineGo_length m s _)

/-- No itemref is silently dropped: one step per spine entry. -/
theorem resolveSpine_length (m : List Item) (s : List Nat) :
    (resolveSpine m s).length = s.length := spineGo_length m s []

theorem spineStep_missing {m : List Item} {seen : List Nat} {i : Nat}
    (h : resolveId m i = .missing) : spineStep m seen i = .missing := by
  unfold spineStep; rw [h]

theorem spineStep_duplicate {m : List Item} {seen : List Nat} {i : Nat}
    (h : resolveId m i = .duplicate) : spineStep m seen i = .duplicate := by
  unfold spineStep; rw [h]

theorem spineStep_read {m : List Item} {seen : List Nat} {i k : Nat}
    (h : spineStep m seen i = .read k) :
    resolveId m i = .item k ∧ contentAt m k = true ∧ memB k seen = false := by
  cases hr : resolveId m i with
  | missing => rw [spineStep_missing hr] at h; cases h
  | duplicate => rw [spineStep_duplicate hr] at h; cases h
  | item k' =>
    rw [spineStep_item hr] at h
    by_cases hc : contentAt m k' = true
    · rw [if_pos hc] at h
      by_cases hs : memB k' seen = true
      · rw [if_pos hs] at h; cases h
      · rw [if_neg hs] at h
        have e : k' = k := Step.read.inj h
        subst e
        exact ⟨rfl, hc, bool_false_of_ne_true hs⟩
    · rw [if_neg hc] at h; cases h

theorem spineStep_repeated {m : List Item} {seen : List Nat} {i k : Nat}
    (h : spineStep m seen i = .repeated k) : memB k seen = true := by
  cases hr : resolveId m i with
  | missing => rw [spineStep_missing hr] at h; cases h
  | duplicate => rw [spineStep_duplicate hr] at h; cases h
  | item k' =>
    rw [spineStep_item hr] at h
    by_cases hc : contentAt m k' = true
    · rw [if_pos hc] at h
      by_cases hs : memB k' seen = true
      · rw [if_pos hs] at h; cases h; exact hs
      · rw [if_neg hs] at h; cases h
    · rw [if_neg hc] at h; cases h

theorem contentAt_some {m : List Item} {k : Nat} (h : contentAt m k = true) :
    ∃ it, nth m k = some it ∧ isContentDoc it.media = true := by
  unfold contentAt at h
  cases hn : nth m k with
  | none => rw [hn] at h; cases h
  | some it => rw [hn] at h; exact ⟨it, rfl, h⟩

theorem spineGo_nth {m : List Item} :
    ∀ (s seen : List Nat) (p : Nat) (st : Step), nth (spineGo m seen s) p = some st →
      ∃ i seen', nth s p = some i ∧ st = spineStep m seen' i
  | [], _, _, _, h => by cases h
  | i :: _, seen, 0, st, h => by
      have : spineStep m seen i = st := by cases h; rfl
      exact ⟨i, seen, rfl, this.symm⟩
  | _ :: s, seen, p + 1, st, h => spineGo_nth s _ p st h

theorem spineGo_nth_of {m : List Item} :
    ∀ (s seen : List Nat) (p i : Nat), nth s p = some i →
      ∃ seen', nth (spineGo m seen s) p = some (spineStep m seen' i)
  | [], _, _, _, h => by cases h
  | i :: _, seen, 0, i', h => by
      have : i = i' := by cases h; rfl
      subst this
      exact ⟨seen, rfl⟩
  | _ :: s, seen, p + 1, i, h => spineGo_nth_of s _ p i h

/-- Never a wrong item: every chapter read is the unique manifest entry whose id is that
spine entry's `idref`, and it is a content document. -/
theorem resolveSpine_read_sound {m : List Item} {s : List Nat} {p k : Nat}
    (h : nth (resolveSpine m s) p = some (.read k)) :
    ∃ i it, nth s p = some i ∧ nth m k = some it ∧ it.id = i ∧
      isContentDoc it.media = true ∧
      ∀ (j : Nat) (it' : Item), nth m j = some it' → it'.id = i → j = k := by
  have ⟨i, seen', hs, hst⟩ := spineGo_nth s [] p _ h
  have ⟨hr, hc, _⟩ := spineStep_read hst.symm
  have ⟨it, hn, hid⟩ := resolveId_item_sound hr
  have ⟨it', hn', hcd⟩ := contentAt_some hc
  rw [hn] at hn'
  cases hn'
  exact ⟨i, it, hs, hn, hid, hcd, fun j it'' hj hj' => resolveId_item_unique hr hj hj'⟩

/-- A missing id is reported at its own position. -/
theorem resolveSpine_missing {m : List Item} {s : List Nat} {p i : Nat}
    (hs : nth s p = some i) (h : ∀ (j : Nat) (it : Item), nth m j = some it → it.id ≠ i) :
    nth (resolveSpine m s) p = some .missing := by
  have ⟨seen', e⟩ := spineGo_nth_of (m := m) s [] p i hs
  rw [spineStep_missing ((resolveId_missing_iff m i).mpr h)] at e
  exact e

/-- A duplicated id is reported at its own position, never resolved. -/
theorem resolveSpine_duplicate {m : List Item} {s : List Nat} {p i : Nat}
    (hs : nth s p = some i)
    (h : ∃ j₁ j₂ it₁ it₂, j₁ < j₂ ∧ nth m j₁ = some it₁ ∧ nth m j₂ = some it₂ ∧
        it₁.id = i ∧ it₂.id = i) :
    nth (resolveSpine m s) p = some .duplicate := by
  have ⟨seen', e⟩ := spineGo_nth_of (m := m) s [] p i hs
  rw [spineStep_duplicate ((resolveId_duplicate_iff m i).mpr h)] at e
  exact e

theorem spineGo_read_not_seen {m : List Item} :
    ∀ (s seen : List Nat) (p k : Nat), nth (spineGo m seen s) p = some (.read k) →
      memB k seen = false
  | [], _, _, _, h => by cases h
  | i :: _, seen, 0, k, h => by
      have : spineStep m seen i = .read k := Option.some.inj h
      exact (spineStep_read this).2.2
  | i :: s, seen, p + 1, k, h => by
      have ih := spineGo_read_not_seen s (stepSeen (spineStep m seen i) seen) p k h
      cases hst : spineStep m seen i with
      | read k' =>
        rw [hst] at ih
        have e : stepSeen (Step.read k') seen = k' :: seen := rfl
        rw [e] at ih
        by_cases hk : k = k'
        · subst hk; rw [memB_head] at ih; cases ih
        · rw [memB_cons_of_ne seen hk] at ih; exact ih
      | missing => rw [hst] at ih; exact ih
      | duplicate => rw [hst] at ih; exact ih
      | foreign _ => rw [hst] at ih; exact ih
      | repeated _ => rw [hst] at ih; exact ih

theorem spineGo_read_nodup {m : List Item} :
    ∀ (s seen : List Nat) (p₁ p₂ k : Nat), p₁ < p₂ →
      nth (spineGo m seen s) p₁ = some (.read k) →
      nth (spineGo m seen s) p₂ = some (.read k) → False
  | [], _, _, _, _, _, h, _ => by cases h
  | _ :: _, _, _, 0, _, hp, _, _ => absurd hp (Nat.not_lt_zero _)
  | i :: s, seen, 0, p₂ + 1, k, _, h₁, h₂ => by
      have hst : spineStep m seen i = .read k := Option.some.inj h₁
      have hn := spineGo_read_not_seen s (stepSeen (spineStep m seen i) seen) p₂ k h₂
      rw [hst] at hn
      have e : stepSeen (Step.read k) seen = k :: seen := rfl
      rw [e, memB_head] at hn
      cases hn
  | _ :: s, seen, p₁ + 1, p₂ + 1, k, hp, h₁, h₂ =>
      spineGo_read_nodup s _ p₁ p₂ k (Nat.lt_of_succ_lt_succ hp) h₁ h₂

/-- No chapter is read twice. -/
theorem resolveSpine_read_nodup {m : List Item} {s : List Nat} {p₁ p₂ k : Nat}
    (h₁ : nth (resolveSpine m s) p₁ = some (.read k))
    (h₂ : nth (resolveSpine m s) p₂ = some (.read k)) : p₁ = p₂ := by
  cases Nat.lt_or_ge p₁ p₂ with
  | inl hlt => exact (spineGo_read_nodup s [] p₁ p₂ k hlt h₁ h₂).elim
  | inr hge =>
    cases Nat.lt_or_ge p₂ p₁ with
    | inl hlt => exact (spineGo_read_nodup s [] p₂ p₁ k hlt h₂ h₁).elim
    | inr hle => exact Nat.le_antisymm hle hge

theorem spineGo_repeated {m : List Item} :
    ∀ (s seen : List Nat) (p k : Nat), nth (spineGo m seen s) p = some (.repeated k) →
      memB k seen = true ∨ ∃ p', p' < p ∧ nth (spineGo m seen s) p' = some (.read k)
  | [], _, _, _, h => by cases h
  | i :: _, seen, 0, k, h => by
      have : spineStep m seen i = .repeated k := Option.some.inj h
      exact Or.inl (spineStep_repeated this)
  | i :: s, seen, p + 1, k, h => by
      have ih := spineGo_repeated s (stepSeen (spineStep m seen i) seen) p k h
      have shift : ∀ (seen' : List Nat), (∃ p', p' < p ∧ nth (spineGo m seen' s) p' =
          some (.read k)) → seen' = stepSeen (spineStep m seen i) seen →
          ∃ p', p' < p + 1 ∧ nth (spineGo m seen (i :: s)) p' = some (.read k) := by
        intro seen' ⟨p', hp, hn⟩ e
        subst e
        exact ⟨p' + 1, Nat.succ_lt_succ hp, hn⟩
      cases hst : spineStep m seen i with
      | read k' =>
        have e : stepSeen (spineStep m seen i) seen = k' :: seen := by rw [hst]; rfl
        rw [e] at ih
        cases ih with
        | inr hex => exact Or.inr (shift _ hex e.symm)
        | inl hm =>
          by_cases hk : k = k'
          · subst hk
            refine Or.inr ⟨0, Nat.succ_pos p, ?_⟩
            show some (spineStep m seen i) = some (Step.read k)
            rw [hst]
          · rw [memB_cons_of_ne seen hk] at hm; exact Or.inl hm
      | missing =>
        have e : stepSeen (spineStep m seen i) seen = seen := by rw [hst]; rfl
        rw [e] at ih
        cases ih with
        | inl hm => exact Or.inl hm
        | inr hex => exact Or.inr (shift _ hex e.symm)
      | duplicate =>
        have e : stepSeen (spineStep m seen i) seen = seen := by rw [hst]; rfl
        rw [e] at ih
        cases ih with
        | inl hm => exact Or.inl hm
        | inr hex => exact Or.inr (shift _ hex e.symm)
      | foreign _ =>
        have e : stepSeen (spineStep m seen i) seen = seen := by rw [hst]; rfl
        rw [e] at ih
        cases ih with
        | inl hm => exact Or.inl hm
        | inr hex => exact Or.inr (shift _ hex e.symm)
      | repeated _ =>
        have e : stepSeen (spineStep m seen i) seen = seen := by rw [hst]; rfl
        rw [e] at ih
        cases ih with
        | inl hm => exact Or.inl hm
        | inr hex => exact Or.inr (shift _ hex e.symm)

/-- A repeated reference is only reported after the same item was read earlier. -/
theorem resolveSpine_repeated {m : List Item} {s : List Nat} {p k : Nat}
    (h : nth (resolveSpine m s) p = some (.repeated k)) :
    ∃ p', p' < p ∧ nth (resolveSpine m s) p' = some (.read k) := by
  cases spineGo_repeated s [] p k h with
  | inl hm => cases hm
  | inr hex => exact hex

/-! ## The spine decision kernel

`spineCode` is the field function of `spine_step`. Codes: `0` read, `1` missing,
`2` duplicate, `3` repeated, `4` foreign, `5` corrupt (the table's answer is out of range
or names an entry with another id: refused, never read). -/

def stepCode : Step → Nat
  | .read _ => 0
  | .missing => 1
  | .duplicate => 2
  | .repeated _ => 3
  | .foreign _ => 4

def spineCode (found dup : Bool) (index count : Nat) (idEq contentDoc seen : Bool) : Nat :=
  if found = false then 1
  else if dup = true then 2
  else if (decide (index < count) && idEq) = false then 5
  else if contentDoc = false then 4
  else if seen = true then 3
  else 0

/-- The kernel reads only a found, unduplicated, in-range entry whose stored id equals the
idref, that is a content document and was not read before; this holds whatever the table
returned. -/
theorem spineCode_read {found dup idEq contentDoc seen : Bool} {index count : Nat}
    (h : spineCode found dup index count idEq contentDoc seen = 0) :
    found = true ∧ dup = false ∧ index < count ∧ idEq = true ∧ contentDoc = true ∧
      seen = false := by
  unfold spineCode at h
  by_cases h1 : found = false
  · rw [if_pos h1] at h; exact absurd h (by decide)
  rw [if_neg h1] at h
  by_cases h2 : dup = true
  · rw [if_pos h2] at h; exact absurd h (by decide)
  rw [if_neg h2] at h
  by_cases h3 : (decide (index < count) && idEq) = false
  · rw [if_pos h3] at h; exact absurd h (by decide)
  rw [if_neg h3] at h
  by_cases h4 : contentDoc = false
  · rw [if_pos h4] at h; exact absurd h (by decide)
  rw [if_neg h4] at h
  by_cases h5 : seen = true
  · rw [if_pos h5] at h; exact absurd h (by decide)
  have h3' := bool_true_of_ne_false h3
  exact ⟨bool_true_of_ne_false h1, bool_false_of_ne_true h2, of_decide_eq_true (band_left h3'),
    band_right h3', bool_true_of_ne_false h4, bool_false_of_ne_true h5⟩

/-- Whether manifest entry `k` has id `i` (the production `items[k].id == idref` check). -/
def idAt (m : List Item) (k i : Nat) : Bool :=
  match nth m k with
  | some it => decide (it.id = i)
  | none => false

/-- The kernel inputs production computes from the built table. -/
def codeAt (m : List Item) (seen : List Nat) (i : Nat) : Nat :=
  match buildIndex m i with
  | none => spineCode false false 0 m.length false false false
  | some p => spineCode true p.2 p.1 m.length (idAt m p.1 i) (contentAt m p.1) (memB p.1 seen)

/-- Fed from the table built over the manifest, the kernel decides every spine entry
exactly as the specification, and never reports `corrupt`. -/
theorem codeAt_eq (m : List Item) (seen : List Nat) (i : Nat) :
    codeAt m seen i = stepCode (spineStep m seen i) := by
  have hb : buildIndex m i = (match firstIndex i m with
      | none => none
      | some j => some (0 + j, decide (1 < countId i m))) := buildFrom_eq i m emptyIndex 0
  unfold codeAt
  rw [hb]
  cases hf : firstIndex i m with
  | none => rw [spineStep_missing (resolveId_none hf)]; rfl
  | some j =>
    have ⟨it, hn, hid⟩ := firstIndex_sound m j hf
    have hpos := countId_pos_of_mem m j it hn hid
    have hidAt : idAt m j i = true := by
      unfold idAt; rw [hn]; exact decide_eq_true hid
    have hlt : decide (j < m.length) = true := decide_eq_true (nth_lt m j it hn)
    show spineCode true (decide (1 < countId i m)) (0 + j) m.length (idAt m (0 + j) i)
        (contentAt m (0 + j)) (memB (0 + j) seen) = _
    rw [Nat.zero_add, hidAt]
    have hr := resolveId_some (m := m) hf
    revert hpos hr
    generalize countId i m = c
    intro hpos hr
    cases c with
    | zero => exact absurd hpos (Nat.lt_irrefl 0)
    | succ c =>
      cases c with
      | zero =>
        rw [if_pos (show 0 + 1 = 1 from rfl)] at hr
        rw [spineStep_item hr]
        unfold spineCode
        rw [hlt]
        cases contentAt m j <;> cases memB j seen <;> rfl
      | succ c =>
        rw [if_neg (show ¬ (c + 1 + 1 = 1) from
          fun e => absurd (Nat.succ.inj e) (succ_ne_zero' c))] at hr
        rw [spineStep_duplicate hr]
        unfold spineCode
        rw [decide_eq_true (show 1 < c + 1 + 1 from Nat.succ_lt_succ (Nat.succ_pos c))]
        rfl

/-- In particular the kernel never reports `corrupt` on a table built from the manifest. -/
theorem codeAt_not_corrupt (m : List Item) (seen : List Nat) (i : Nat) : codeAt m seen i ≠ 5 := by
  rw [codeAt_eq]
  cases spineStep m seen i with
  | read _ => exact (show (0 : Nat) ≠ 5 by decide)
  | missing => exact (show (1 : Nat) ≠ 5 by decide)
  | duplicate => exact (show (2 : Nat) ≠ 5 by decide)
  | foreign _ => exact (show (4 : Nat) ≠ 5 by decide)
  | repeated _ => exact (show (3 : Nat) ≠ 5 by decide)

/-! ## Paths inside the container (EPUB 3.3 OCF, URL Standard 1.3, RFC 3986 5.2.4)

Bytes are `Nat`s. -/

/-- `href_byte_class`: `1` the separator `/`, `2` `%`, `3` `#` or `?` (the path ends),
`0` any other byte. -/
def byteClass (b : Nat) : Nat :=
  if b = 47 then 1 else if b = 37 then 2 else if b = 35 ∨ b = 63 then 3 else 0

/-- The path of a URL string: everything before the first `?` or `#`. -/
def pathPart : List Nat → List Nat
  | [] => []
  | b :: bs => if byteClass b = 3 then [] else b :: pathPart bs

theorem pathPart_clean : ∀ (l : List Nat) (j b : Nat), nth (pathPart l) j = some b →
    byteClass b ≠ 3
  | [], _, _, h => by cases h
  | b :: bs, j, b', h => by
      by_cases hc : byteClass b = 3
      · have e : pathPart (b :: bs) = [] := if_pos hc
        rw [e] at h; cases h
      · have e : pathPart (b :: bs) = b :: pathPart bs := if_neg hc
        rw [e] at h
        cases j with
        | zero => cases h; exact hc
        | succ j => exact pathPart_clean bs j b' h

/-- Split on `/`; `cur` holds the current segment reversed. -/
def splitGo (cur : List Nat) : List Nat → List (List Nat)
  | [] => [cur.reverse]
  | b :: bs => if byteClass b = 1 then cur.reverse :: splitGo [] bs else splitGo (b :: cur) bs

def segments (p : List Nat) : List (List Nat) := splitGo [] p

/-- `pct_hex`: the value of an ASCII hex digit. -/
def hexVal (b : Nat) : Option Nat :=
  if 48 ≤ b ∧ b ≤ 57 then some (b - 48)
  else if 65 ≤ b ∧ b ≤ 70 then some (b - 55)
  else if 97 ≤ b ∧ b ≤ 102 then some (b - 87)
  else none

def hexFields (b : Nat) : Bool × Nat :=
  match hexVal b with
  | some v => (true, v)
  | none => (false, 0)

theorem hexVal_lt {b v : Nat} (h : hexVal b = some v) : v < 16 := by
  unfold hexVal at h
  by_cases h1 : 48 ≤ b ∧ b ≤ 57
  · rw [if_pos h1] at h
    cases h
    exact Nat.lt_of_le_of_lt (Nat.sub_le_sub_right h1.2 48) (by decide)
  rw [if_neg h1] at h
  by_cases h2 : 65 ≤ b ∧ b ≤ 70
  · rw [if_pos h2] at h
    cases h
    exact Nat.lt_of_le_of_lt (Nat.sub_le_sub_right h2.2 55) (by decide)
  rw [if_neg h2] at h
  by_cases h3 : 97 ≤ b ∧ b ≤ 102
  · rw [if_pos h3] at h
    cases h
    exact Nat.lt_of_le_of_lt (Nat.sub_le_sub_right h3.2 87) (by decide)
  rw [if_neg h3] at h
  cases h

theorem hexFields_spec (b : Nat) :
    (hexFields b).1 = true → hexVal b = some (hexFields b).2 := by
  unfold hexFields
  cases hexVal b with
  | none => intro h; cases h
  | some v => intro _; rfl

/-- Percent-decoding state: after `%`, and after `%` and one hex digit (raw byte, value). -/
inductive Pct where
  | normal
  | pct
  | pctHex (raw v : Nat)

/-- URL 1.3 percent-decode: `%XY` with two hex digits is one byte; any other `%` stays. -/
def pctGo : Pct → List Nat → List Nat
  | .normal, [] => []
  | .pct, [] => [37]
  | .pctHex raw _, [] => [37, raw]
  | .normal, b :: rest => if b = 37 then pctGo .pct rest else b :: pctGo .normal rest
  | .pct, b :: rest =>
    match hexVal b with
    | some v => pctGo (.pctHex b v) rest
    | none => if b = 37 then 37 :: pctGo .pct rest else 37 :: b :: pctGo .normal rest
  | .pctHex raw v, b :: rest =>
    match hexVal b with
    | some w => (16 * v + w) :: pctGo .normal rest
    | none =>
      if b = 37 then 37 :: raw :: pctGo .pct rest else 37 :: raw :: b :: pctGo .normal rest

def pctDecode (l : List Nat) : List Nat := pctGo .normal l

/-- A path without `%` is its own decoding. -/
theorem pctDecode_plain : ∀ (l : List Nat), (∀ (j b : Nat), nth l j = some b → b ≠ 37) →
    pctDecode l = l
  | [], _ => rfl
  | b :: rest, h => by
      have hb : b ≠ 37 := h 0 b rfl
      have e : pctDecode (b :: rest) = b :: pctDecode rest := if_neg hb
      rw [e, pctDecode_plain rest (fun j b' hj => h (j + 1) b' hj)]

def pending : Pct → Nat
  | .normal => 0
  | .pct => 1
  | .pctHex _ _ => 2

theorem pctGo_length : ∀ (l : List Nat) (st : Pct), (pctGo st l).length ≤ l.length + pending st
  | [], .normal => Nat.le_refl _
  | [], .pct => Nat.le_refl _
  | [], .pctHex _ _ => Nat.le_refl _
  | b :: rest, .normal => by
      show (if b = 37 then pctGo .pct rest else b :: pctGo .normal rest).length ≤ rest.length + 1
      by_cases hb : b = 37
      · rw [if_pos hb]; exact pctGo_length rest .pct
      · rw [if_neg hb]; exact Nat.succ_le_succ (pctGo_length rest .normal)
  | b :: rest, .pct => by
      show (match hexVal b with
        | some v => pctGo (.pctHex b v) rest
        | none => if b = 37 then 37 :: pctGo .pct rest else 37 :: b :: pctGo .normal rest).length
        ≤ rest.length + 1 + 1
      cases hexVal b with
      | some v => exact pctGo_length rest (.pctHex b v)
      | none =>
        by_cases hb : b = 37
        · rw [if_pos hb]; exact Nat.succ_le_succ (pctGo_length rest .pct)
        · rw [if_neg hb]
          exact Nat.succ_le_succ (Nat.succ_le_succ (pctGo_length rest .normal))
  | b :: rest, .pctHex raw v => by
      show (match hexVal b with
        | some w => (16 * v + w) :: pctGo .normal rest
        | none => if b = 37 then 37 :: raw :: pctGo .pct rest
            else 37 :: raw :: b :: pctGo .normal rest).length ≤ rest.length + 1 + 2
      cases hexVal b with
      | some w =>
        exact Nat.le_trans (Nat.succ_le_succ (pctGo_length rest .normal))
          (Nat.succ_le_succ (Nat.le_add_right _ _))
      | none =>
        by_cases hb : b = 37
        · rw [if_pos hb]
          exact Nat.succ_le_succ (Nat.succ_le_succ (pctGo_length rest .pct))
        · rw [if_neg hb]
          exact Nat.succ_le_succ (Nat.succ_le_succ (Nat.succ_le_succ
            (pctGo_length rest .normal)))

/-- Decoding never lengthens a path. -/
theorem pctDecode_length_le (l : List Nat) : (pctDecode l).length ≤ l.length :=
  pctGo_length l .normal

/-- `path_segment_kind`: `1` the segment `.`, `2` the segment `..`, `0` any other. -/
def segKindFields (len b0 b1 : Nat) : Nat :=
  if len = 1 ∧ b0 = 46 then 1 else if len = 2 ∧ b0 = 46 ∧ b1 = 46 then 2 else 0

def segKind (seg : List Nat) : Nat :=
  segKindFields seg.length ((nth seg 0).getD 0) ((nth seg 1).getD 0)

theorem segKind_one {seg : List Nat} (h : segKind seg = 1) : seg = [46] := by
  unfold segKind segKindFields at h
  by_cases h1 : seg.length = 1 ∧ (nth seg 0).getD 0 = 46
  · cases seg with
    | nil => exact absurd h1.1 (by decide)
    | cons a t =>
      cases t with
      | nil =>
        have ha : a = 46 := h1.2
        rw [ha]
      | cons _ t' => exact absurd (Nat.succ.inj h1.1) (succ_ne_zero' _)
  · rw [if_neg h1] at h
    by_cases h2 : seg.length = 2 ∧ (nth seg 0).getD 0 = 46 ∧ (nth seg 1).getD 0 = 46
    · rw [if_pos h2] at h; exact absurd h (by decide)
    · rw [if_neg h2] at h; exact absurd h (by decide)

theorem segKind_two {seg : List Nat} (h : segKind seg = 2) : seg = [46, 46] := by
  unfold segKind segKindFields at h
  by_cases h1 : seg.length = 1 ∧ (nth seg 0).getD 0 = 46
  · rw [if_pos h1] at h; exact absurd h (by decide)
  · rw [if_neg h1] at h
    by_cases h2 : seg.length = 2 ∧ (nth seg 0).getD 0 = 46 ∧ (nth seg 1).getD 0 = 46
    · cases seg with
      | nil => exact absurd h2.1 (by decide)
      | cons a t =>
        cases t with
        | nil => exact absurd h2.1 (show ¬ (1 = 2) by decide)
        | cons b t' =>
          cases t' with
          | nil =>
            have ha : a = 46 := h2.2.1
            have hb : b = 46 := h2.2.2
            rw [ha, hb]
          | cons _ _ =>
            exact absurd (Nat.succ.inj (Nat.succ.inj h2.1)) (succ_ne_zero' _)
    · rw [if_neg h2] at h; exact absurd h (by decide)

/-- `acc` reversed onto `out`. -/
def revOnto {α : Type} : List α → List α → List α
  | [], out => out
  | x :: xs, out => revOnto xs (x :: out)

/-- RFC 3986 5.2.4 on a segment list; `acc` is the output so far, last segment first. -/
def removeDotsGo : List (List Nat) → List (List Nat) → List (List Nat)
  | acc, [] => revOnto acc []
  | acc, s :: rest =>
    if segKind s = 1 then removeDotsGo acc rest
    else if segKind s = 2 then removeDotsGo acc.tail rest
    else removeDotsGo (s :: acc) rest

def removeDots (segs : List (List Nat)) : List (List Nat) := removeDotsGo [] segs

/-- Every segment is an ordinary one (neither `.` nor `..`). -/
def allPlain : List (List Nat) → Bool
  | [] => true
  | s :: r => decide (segKind s = 0) && allPlain r

theorem allPlain_revOnto : ∀ (acc out : List (List Nat)), allPlain acc = true →
    allPlain out = true → allPlain (revOnto acc out) = true
  | [], _, _, ho => ho
  | s :: acc, out, ha, ho =>
      allPlain_revOnto acc (s :: out) (band_right ha) (by
        show (decide (segKind s = 0) && allPlain out) = true
        rw [band_left ha, ho]; rfl)

theorem allPlain_tail : ∀ (acc : List (List Nat)), allPlain acc = true → allPlain acc.tail = true
  | [], h => h
  | _ :: _, h => band_right h

theorem removeDotsGo_clean : ∀ (segs acc : List (List Nat)), allPlain acc = true →
    allPlain (removeDotsGo acc segs) = true
  | [], acc, ha => allPlain_revOnto acc [] ha rfl
  | s :: rest, acc, ha => by
      show allPlain (if segKind s = 1 then removeDotsGo acc rest
        else if segKind s = 2 then removeDotsGo acc.tail rest
        else removeDotsGo (s :: acc) rest) = true
      by_cases h1 : segKind s = 1
      · rw [if_pos h1]; exact removeDotsGo_clean rest acc ha
      · rw [if_neg h1]
        by_cases h2 : segKind s = 2
        · rw [if_pos h2]; exact removeDotsGo_clean rest acc.tail (allPlain_tail acc ha)
        · rw [if_neg h2]
          apply removeDotsGo_clean rest (s :: acc)
          show (decide (segKind s = 0) && allPlain acc) = true
          have h0 : segKind s = 0 := by
            unfold segKind segKindFields at h1 h2 ⊢
            by_cases c1 : s.length = 1 ∧ (nth s 0).getD 0 = 46
            · rw [if_pos c1] at h1; exact absurd rfl h1
            · rw [if_neg c1] at h1 h2 ⊢
              by_cases c2 : s.length = 2 ∧ (nth s 0).getD 0 = 46 ∧ (nth s 1).getD 0 = 46
              · rw [if_pos c2] at h2; exact absurd rfl h2
              · rw [if_neg c2]
          rw [decide_eq_true h0, ha]; rfl

/-- No `.` or `..` segment survives to the ZIP lookup. -/
theorem removeDots_clean (segs : List (List Nat)) : allPlain (removeDots segs) = true :=
  removeDotsGo_clean segs [] rfl

theorem removeDotsGo_plain : ∀ (segs acc : List (List Nat)), allPlain segs = true →
    removeDotsGo acc segs = revOnto acc segs
  | [], _, _ => rfl
  | s :: rest, acc, h => by
      have h0 : segKind s = 0 := of_decide_eq_true (band_left h)
      show (if segKind s = 1 then removeDotsGo acc rest
        else if segKind s = 2 then removeDotsGo acc.tail rest
        else removeDotsGo (s :: acc) rest) = revOnto (s :: acc) rest
      rw [h0, if_neg (by decide), if_neg (by decide)]
      exact removeDotsGo_plain rest (s :: acc) (band_right h)

/-- A path without dot segments is unchanged. -/
theorem removeDots_plain (segs : List (List Nat)) (h : allPlain segs = true) :
    removeDots segs = segs :=
  removeDotsGo_plain segs [] h

/-- `..` at the container root goes nowhere: a path never climbs above the root. -/
theorem removeDots_parent_at_root (segs : List (List Nat)) :
    removeDots ([46, 46] :: segs) = removeDots segs := by
  show (if segKind [46, 46] = 1 then removeDotsGo [] segs
    else if segKind [46, 46] = 2 then removeDotsGo ([] : List (List Nat)).tail segs
    else removeDotsGo [[46, 46]] segs) = removeDotsGo [] segs
  rw [if_neg (by decide), if_pos (by decide)]
  rfl

/-- The directory segments of the package document's path (its own name dropped). -/
def dirSegs (opfPath : List Nat) : List (List Nat) := (segments opfPath).dropLast

def headSlash : List Nat → Bool
  | [] => false
  | b :: _ => decide (b = 47)

/-- The ZIP entry, as segments, that a manifest `href` names (EPUB 3.3 OCF): the
path part, resolved against the package directory (or the container root for a
path-absolute href), segments percent-decoded, dot segments removed. -/
def resolveHref (opfPath href : List Nat) : List (List Nat) :=
  if headSlash (pathPart href) = true then
    removeDots ((segments (pathPart href).tail).map pctDecode)
  else
    removeDots ((dirSegs opfPath ++ segments (pathPart href)).map pctDecode)

/-- A path-absolute href names the same entry whatever the package directory, including
a package at the container root. -/
theorem resolveHref_absolute {href : List Nat} (h : headSlash (pathPart href) = true)
    (a b : List Nat) : resolveHref a href = resolveHref b href := by
  unfold resolveHref
  rw [if_pos h, if_pos h]

theorem resolveHref_clean (opfPath href : List Nat) :
    allPlain (resolveHref opfPath href) = true := by
  unfold resolveHref
  by_cases h : headSlash (pathPart href) = true
  · rw [if_pos h]; exact removeDots_clean _
  · rw [if_neg h]; exact removeDots_clean _

/-! ## Bounds (the reader's own limits)

`textBudget` is the field function of `epub_text_budget`: the joined text grows by the
separator and the chapter only when the result stays within the budget. Chapter `idx > 0`
is preceded by the two-byte separator. -/

def sepLen (idx : Nat) : Nat := if idx = 0 then 0 else 2

def textBudget (out sep md max : Nat) : Bool × Nat :=
  if out + sep + md ≤ max then (true, out + sep + md) else (false, out)

theorem textBudget_accept {out sep md max : Nat} (h : (textBudget out sep md max).1 = true) :
    (textBudget out sep md max).2 = out + sep + md ∧ out + sep + md ≤ max := by
  unfold textBudget at h ⊢
  by_cases hc : out + sep + md ≤ max
  · rw [if_pos hc]; exact ⟨rfl, hc⟩
  · rw [if_neg hc] at h; cases h

theorem textBudget_refuse {out sep md max : Nat} (h : (textBudget out sep md max).1 = false) :
    max < out + sep + md := by
  unfold textBudget at h
  by_cases hc : out + sep + md ≤ max
  · rw [if_pos hc] at h; cases h
  · exact Nat.lt_of_not_le hc

/-- The length of the joined text of chapters of lengths `mds` (after `out` bytes). -/
def totalFrom : Nat → Nat → List Nat → Nat
  | out, _, [] => out
  | out, idx, md :: rest => totalFrom (out + sepLen idx + md) (idx + 1) rest

/-- The production loop: one budget check per chapter, refusing the whole book on the
first chapter that does not fit. -/
def renderGo (max : Nat) : Nat → Nat → List Nat → Option Nat
  | out, _, [] => some out
  | out, idx, md :: rest =>
    if (textBudget out (sepLen idx) md max).1 = true then
      renderGo max (textBudget out (sepLen idx) md max).2 (idx + 1) rest
    else none

theorem totalFrom_ge : ∀ (mds : List Nat) (out idx : Nat), out ≤ totalFrom out idx mds
  | [], _, _ => Nat.le_refl _
  | md :: rest, out, idx =>
      Nat.le_trans (Nat.le_trans (Nat.le_add_right out (sepLen idx))
        (Nat.le_add_right _ md)) (totalFrom_ge rest _ (idx + 1))

theorem renderGo_eq (max : Nat) : ∀ (mds : List Nat) (out idx : Nat), out ≤ max →
    renderGo max out idx mds =
      if totalFrom out idx mds ≤ max then some (totalFrom out idx mds) else none
  | [], out, _, h => by
      show some out = if out ≤ max then some out else none
      rw [if_pos h]
  | md :: rest, out, idx, h => by
      show (if (textBudget out (sepLen idx) md max).1 = true then
          renderGo max (textBudget out (sepLen idx) md max).2 (idx + 1) rest else none) =
        if totalFrom (out + sepLen idx + md) (idx + 1) rest ≤ max then
          some (totalFrom (out + sepLen idx + md) (idx + 1) rest) else none
      by_cases hc : out + sepLen idx + md ≤ max
      · have e : textBudget out (sepLen idx) md max = (true, out + sepLen idx + md) := if_pos hc
        rw [e, if_pos rfl]
        exact renderGo_eq max rest _ (idx + 1) hc
      · have e : textBudget out (sepLen idx) md max = (false, out) := if_neg hc
        rw [e, if_neg (fun hh => Bool.noConfusion hh)]
        have hn : ¬ totalFrom (out + sepLen idx + md) (idx + 1) rest ≤ max := fun ht =>
          hc (Nat.le_trans (totalFrom_ge rest _ (idx + 1)) ht)
        rw [if_neg hn]

/-- The text is returned exactly when the whole joined text fits the budget, and then it is
the whole text; nothing partial is ever returned. -/
theorem render_spec (max : Nat) (mds : List Nat) :
    renderGo max 0 0 mds = if totalFrom 0 0 mds ≤ max then some (totalFrom 0 0 mds) else none :=
  renderGo_eq max mds 0 0 (Nat.zero_le max)

/-- `zip_entry_bound`: an entry is accepted only when its declared size and the bytes
actually read are both within the bound. -/
def entryBoundOk (declared read max : Nat) : Bool := decide (declared ≤ max) && decide (read ≤ max)

theorem entryBoundOk_spec {declared read max : Nat} (h : entryBoundOk declared read max = true) :
    declared ≤ max ∧ read ≤ max :=
  ⟨of_decide_eq_true (band_left h), of_decide_eq_true (band_right h)⟩

theorem entryBoundOk_of {declared read max : Nat} (hd : declared ≤ max) (hr : read ≤ max) :
    entryBoundOk declared read max = true := by
  unfold entryBoundOk
  rw [decide_eq_true hd, decide_eq_true hr]
  rfl

/-! ## Examples -/

/-- `ch1` (id 1) and `ch2` (id 2) are XHTML, id 2 appears twice, id 3 is an image. -/
def exManifest : List Item :=
  [⟨1, [97], 1⟩, ⟨2, [98], 1⟩, ⟨2, [99], 1⟩, ⟨3, [100], 9⟩, ⟨5, [101], 1⟩]

theorem examples :
    resolveId exManifest 1 = .item 0 ∧
    resolveId exManifest 2 = .duplicate ∧
    resolveId exManifest 4 = .missing ∧
    resolveId exManifest 5 = .item 4 ∧
    lookupIndex (buildIndex exManifest) 2 = .duplicate ∧
    lookupIndex (buildIndex exManifest) 5 = .item 4 ∧
    resolveSpine exManifest [1, 4, 2, 3, 5, 1] =
      [.read 0, .missing, .duplicate, .foreign 3, .read 4, .repeated 0] ∧
    codeAt exManifest [] 1 = 0 ∧ codeAt exManifest [0] 1 = 3 ∧ codeAt exManifest [] 3 = 4 ∧
    spineCode true false 7 5 true true false = 5 ∧
    spineCode true false 1 5 false true false = 5 ∧
    roleOf [(1, true), (2, true), (4, true)] = 4 ∧
    roleOf [(1, true), (3, true), (5, true)] = 5 ∧
    roleOf [(1, true), (3, true), (4, true)] = 6 ∧
    roleOf [(1, true), (2, true), (4, false)] = 6 ∧
    roleOf [(1, true), (2, true), (4, true), (4, true)] = 6 ∧
    itemAccept true true 0 = false ∧
    -- "O/c.opf" + "a.x" -> "O/a.x"
    resolveHref [79, 47, 99] [97, 46, 120] = [[79], [97, 46, 120]] ∧
    -- "/a.x" names "a.x" at the root, with the package at the root or not
    resolveHref [99] [47, 97, 46, 120] = [[97, 46, 120]] ∧
    resolveHref [79, 47, 99] [47, 97, 46, 120] = [[97, 46, 120]] ∧
    -- "O/P/c.opf" + "../T/a.x" -> "O/T/a.x"
    resolveHref [79, 47, 80, 47, 99] [46, 46, 47, 84, 47, 97, 46, 120] =
      [[79], [84], [97, 46, 120]] ∧
    -- "c.opf" + "../../a" -> "a"
    resolveHref [99] [46, 46, 47, 46, 46, 47, 97] = [[97]] ∧
    -- "a%20b" -> "a b";  "a%2Fb" stays one segment;  "a.x#s" and "a.x?q" -> "a.x"
    resolveHref [99] [97, 37, 50, 48, 98] = [[97, 32, 98]] ∧
    resolveHref [99] [97, 37, 50, 70, 98] = [[97, 47, 98]] ∧
    resolveHref [99] [97, 46, 120, 35, 115] = [[97, 46, 120]] ∧
    resolveHref [99] [97, 46, 120, 63, 113] = [[97, 46, 120]] ∧
    -- "%2E%2E/a" is a dot segment once decoded
    resolveHref [79, 47, 99] [37, 50, 69, 37, 50, 69, 47, 97] = [[97]] ∧
    pctDecode [37] = [37] ∧ pctDecode [37, 52] = [37, 52] ∧
    pctDecode [37, 52, 103] = [37, 52, 103] ∧ pctDecode [37, 37, 52, 49] = [37, 65] ∧
    pctDecode [37, 52, 49, 37, 54, 97] = [65, 106] ∧
    hexFields 70 = (true, 15) ∧ hexFields 71 = (false, 0) ∧
    segKindFields 1 46 0 = 1 ∧ segKindFields 2 46 46 = 2 ∧ segKindFields 2 46 97 = 0 ∧
    textBudget 10 2 5 17 = (true, 17) ∧ textBudget 10 2 5 16 = (false, 10) ∧
    renderGo 10 0 0 [4, 4] = some 10 ∧ renderGo 9 0 0 [4, 4] = none ∧
    entryBoundOk 5 6 5 = false ∧ entryBoundOk 5 5 5 = true := by
  refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_,
    ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_,
    ?_⟩ <;> decide

end EpubSpec
