/-!
# Specification: PDF cross-reference streams, object streams and PNG predictors (G18)

Each section states, in plain Lean over `Nat`, `Bool` and `List`, what the shipped Rust in
`src/pdf/parser/` must compute and, as proved theorems, what it must never do. The
specification is written before the implementation and is not edited to fit the code:
`PdfProofs.lean` is to prove that the Charon/Aeneas extraction of `proof/extract_pdf.rs`
(which production includes through `#[path]` and calls on its decision path) refines the
`...Fields` functions below. No section imports the extraction, so a specification cannot
be weakened by the code it judges.

Every declaration here rests on the kernel alone (no `propext`, `Quot.sound` or `Classical.choice`);
`proof/audit.py` checks this with an empty allowlist.

Sources (ISO 32000-1:2008, "PDF 1.7"; PNG = ISO/IEC 15948:2004 / W3C PNG 2nd ed.):
* 7.3.8.1 (stream extent): the data are the `/Length` bytes that follow the end-of-line
  marker after the keyword `stream`.
* 7.3.10 (indirect objects): "An indirect reference to an undefined object shall not be
  considered an error by a conforming reader; it shall be treated as a reference to the
  null object."
* 7.5.4 / 7.5.6 (cross-reference sections, incremental updates): a later section's entry
  for an object supersedes every earlier one, a free (`f`, type 0) entry included; the
  trailer's `/Prev` is the byte offset of the previous section.
* 7.5.7 (object streams): `/N` is the number of compressed objects, `/First` the byte
  offset in the decoded stream of the first object; the stream begins with `N` pairs of
  integers "object number, byte offset" where "the byte offset shall be relative to the
  first object" and "the byte offsets shall be in increasing order". The generation
  number of an object stream and of every object in it is 0, and an object stream shall
  not itself be stored in an object stream.
* 7.5.8.2, Table 17 (`/W`, `/Index`): an entry is `W[0] + W[1] + W[2]` bytes; "fields
  requiring more than one byte are stored with the high-order byte first"; "a value of
  zero for an element in the W array indicates that the corresponding field shall not be
  present in the stream, and the default value shall be used"; "if the first element is
  zero, the type field shall not be present, and shall default to type 1". `/Index`
  defaults to `[0 Size]`.
* 7.5.8.3, Table 18 (entry types): type 0 free (next free object, generation), type 1
  in use (byte offset, generation; generation defaults to 0), type 2 compressed (object
  number of the object stream, whose generation "shall be implicitly 0", and the index
  of the object within the stream); "any other value shall be interpreted as a reference
  to the null object".
* 7.4.4.4, Table 8 (`/DecodeParms` of FlateDecode/LZWDecode): `/Predictor` 1 none,
  2 TIFF Predictor 2, 10 to 15 PNG (the per-row tag byte selects the algorithm);
  `/Colors` default 1, `/BitsPerComponent` (1, 2, 4, 8, 16) default 8, `/Columns`
  default 1.
* PNG 9.2 / 9.4: each scanline is preceded by a filter-type byte 0 None, 1 Sub, 2 Up,
  3 Average, 4 Paeth (anything else is invalid); "unsigned arithmetic modulo 256";
  `a` is the byte `bpp` positions to the left (0 before the line), `b` the byte above,
  `c` the byte above `a`; "for the first scanline the prior scanline is all zeros";
  `bpp` is the bytes per complete pixel, rounded up to 1. The Paeth predictor computes
  `p = a + b - c` and picks the nearest of `a`, `b`, `c` to `p`, ties in that order.

Deliberate readings:
* A cross-reference entry whose type-1 offset is not inside the file, whose generation does
  not match the reference, or whose type is unknown resolves to `absent` (the null object
  of 7.3.10); nothing is read from outside the file.
* A type-2 reference resolves only when the object stream's own entry is an in-file
  type-1 entry with generation 0, the stream is not the object itself, the stream's header
  was accepted, and the index is below its `/N`. Nested object streams never resolve.
* A field value is computed exactly; one that does not fit in 64 bits is refused (no
  offset in a file the reader can hold needs more), never wrapped.
* A `/W` whose row length is 0 is refused: with no bytes per entry, `/Size` alone would
  drive the number of entries (a hang). Entries stop at the first row that is not
  complete in the decoded data, so the entries decoded never outnumber its bytes.
* The object-stream header's `N` integer pairs need at least `4N - 1` bytes before
  `/First` (each integer at least one digit, separated by white space), so `4N ≤ First + 1`
  is required before anything is allocated for `N`.
* An object's extent in an object stream runs from `First + offset` to the next object's
  `First + offset` (or the end of the stream); every accepted extent lies in
  `[First, length)`, so no object overlaps the header.
* PNG: a trailing partial row is not decoded; an invalid filter type refuses the stream.
  `/Predictor` values other than 1, 2 and 10 to 15 refuse the stream.
* The `/Prev` chain is followed only to in-file offsets not yet visited, so it cannot
  loop and has at most `fileLen` sections.

Kernels. Each decision is a loop-free kernel of `proof/extract_pdf.rs` over
`bool`/`u8`/`u32`/`u64` that production calls; `PdfProofs.lean` proves each equal to its
field function here (with `Nat` inputs being the words' values). The loops stay in Rust
and are exactly the recursions `beFold`, `subsection`/`sectionFrom`, `lookup`,
`reconRowGo`, `pngDecodeGo` and `chainOk` below.
* `xref_be_step(acc: u64, byte: u8) -> (bool, u64)` is `beStepFields`.
* `xref_row_len(w1: u64, w2: u64, w3: u64) -> (bool, u64)` is `rowLenFields`.
* `row_start(index: u64, row_len: u64, data_len: u64) -> (bool, u64)` is
  `rowStartFields` (cross-reference rows, and PNG rows with `row_len = rowBytes + 1`).
* `xref_entry_type(w1_zero: bool, f1: u64) -> u8` is `entryTypeCode`.
* `xref_offset_ok(offset: u64, file_len: u64) -> bool` is `offsetOk`.
* `objstm_ref_ok(stream_direct: bool, is_self: bool, has_n: bool, n: u64, index: u64)
  -> bool` is `objstmRefOk`.
* `objstm_header_ok(first: u64, len: u64, n: u64) -> bool` is `objstmHeaderFields`.
* `objstm_span(first: u64, off: u64, has_next: bool, next_off: u64, len: u64)
  -> (bool, u64, u64)` is `spanFields`.
* `xref_prev_ok(offset: u64, file_len: u64, seen: bool) -> bool` is `prevOkFields`.
* `stream_extent(start: u64, length: u64, file_len: u64) -> (bool, u64)` is
  `streamExtentFields`.
* `predictor_kind(p: u64) -> u8` is `predictorKind`.
* `png_geometry(colors: u64, bpc: u64, columns: u64) -> (bool, u64, u64)` is
  `pngGeometryFields`.
* `png_recon_byte(tag: u8, x: u8, a: u8, b: u8, c: u8) -> (bool, u8)` is `reconFields`.
-/

namespace PdfSpec

/-! ## Arithmetic, Bool and list facts without `propext` -/

namespace Arith

theorem band_left : ∀ {a b : Bool}, (a && b) = true → a = true
  | true, _, _ => rfl
  | false, _, h => h

theorem band_right : ∀ {a b : Bool}, (a && b) = true → b = true
  | true, _, h => h
  | false, _, h => Bool.noConfusion h

theorem band_intro : ∀ {a b : Bool}, a = true → b = true → (a && b) = true
  | true, true, _, _ => rfl

theorem bnot_true : ∀ {a : Bool}, (!a) = true → a = false
  | false, _ => rfl
  | true, h => Bool.noConfusion h

theorem sub_add_cancel : ∀ (n s : Nat), s ≤ n → n - s + s = n
  | _, 0, _ => rfl
  | 0, _ + 1, h => absurd h (Nat.not_succ_le_zero _)
  | n + 1, s + 1, h => by
      rw [Nat.succ_sub_succ, ← Nat.add_assoc, sub_add_cancel n s (Nat.le_of_succ_le_succ h)]

theorem add_sub_cancel : ∀ (a b : Nat), a + b - b = a
  | _, 0 => rfl
  | a, b + 1 => by rw [← Nat.add_assoc, Nat.succ_sub_succ]; exact add_sub_cancel a b

theorem sub_eq_zero_of_le : ∀ (a b : Nat), a ≤ b → a - b = 0
  | 0, b, _ => Nat.zero_sub b
  | _ + 1, 0, h => absurd h (Nat.not_succ_le_zero _)
  | a + 1, b + 1, h => by
      rw [Nat.succ_sub_succ]; exact sub_eq_zero_of_le a b (Nat.le_of_succ_le_succ h)

theorem le_of_add_le_add_right : ∀ {a b : Nat} (c : Nat), a + c ≤ b + c → a ≤ b
  | _, _, 0, h => h
  | _, _, c + 1, h => le_of_add_le_add_right c (Nat.le_of_succ_le_succ h)

theorem sub_le_of_le_add {a b c : Nat} (h : a ≤ b + c) : a - c ≤ b := by
  cases Nat.le_total c a with
  | inl hca =>
      apply le_of_add_le_add_right c
      rw [sub_add_cancel a c hca]
      exact h
  | inr hac =>
      rw [sub_eq_zero_of_le a c hac]
      exact Nat.zero_le b

/-- `b ≤ c - a` from `a + b ≤ c`. -/
theorem le_sub_of_add_le {a b c : Nat} (h : a + b ≤ c) : b ≤ c - a := by
  have hac : a ≤ c := Nat.le_trans (Nat.le_add_right a b) h
  apply le_of_add_le_add_right a
  rw [sub_add_cancel c a hac, Nat.add_comm]
  exact h

/-- `a - b < c` from `b ≤ a` and `a < b + c`. -/
theorem sub_lt_of_lt_add {a b c : Nat} (hba : b ≤ a) (h : a < b + c) : a - b < c := by
  apply Nat.lt_of_succ_le
  apply le_of_add_le_add_right b
  rw [Nat.succ_add, sub_add_cancel a b hba, Nat.add_comm c b]
  exact h

theorem add_sub_cancel_left (a b : Nat) : a + b - a = b := by
  rw [Nat.add_comm]; exact add_sub_cancel b a

theorem right_distrib (a b c : Nat) : (a + b) * c = a * c + b * c := by
  rw [Nat.mul_comm, Nat.left_distrib, Nat.mul_comm c a, Nat.mul_comm c b]

theorem go_div_mod (k : Nat) (hk : 0 < k) : ∀ (fuel x : Nat) (h₁ h₂ : x < fuel),
    k * Nat.div.go k hk fuel x h₁ + Nat.modCore.go k hk fuel x h₂ = x
  | 0, _, h, _ => absurd h (Nat.not_lt_zero _)
  | f + 1, x, h₁, h₂ => by
      rw [Nat.div.go.eq_def, Nat.modCore.go.eq_def]
      dsimp only
      by_cases hx : k ≤ x
      · rw [dif_pos hx, dif_pos hx, Nat.mul_succ, Nat.add_right_comm,
          go_div_mod k hk f (x - k) _ _]
        exact sub_add_cancel x k hx
      · rw [dif_neg hx, dif_neg hx, Nat.mul_zero, Nat.zero_add]

/-- Euclidean division, `Nat.div_add_mod` without `propext`. -/
theorem div_add_mod (n k : Nat) (hk : 0 < k) : k * (n / k) + n % k = n := by
  have hd : n / k = Nat.div.go k hk (n + 1) n (Nat.lt_succ_self n) := by
    show Nat.div n k = _
    rw [Nat.div.eq_def, dif_pos hk]
  cases n with
  | zero =>
      rw [hd, Nat.div.go.eq_def]
      dsimp only
      rw [dif_neg (Nat.not_le_of_gt hk), Nat.mul_zero]
      rfl
  | succ m =>
      by_cases hkn : k ≤ m + 1
      · have hm : (m + 1) % k = Nat.modCore.go k hk (m + 2) (m + 1) (Nat.lt_succ_self _) := by
          show Nat.mod (m + 1) k = _
          rw [Nat.mod.eq_def]
          show (if k ≤ m + 1 then Nat.modCore (m + 1) k else m + 1) = _
          rw [if_pos hkn, Nat.modCore.eq_def, dif_pos hk]
        rw [hd, hm]
        exact go_div_mod k hk _ _ _ _
      · have hm : (m + 1) % k = m + 1 := by
          show Nat.mod (m + 1) k = _
          rw [Nat.mod.eq_def]
          show (if k ≤ m + 1 then Nat.modCore (m + 1) k else m + 1) = _
          rw [if_neg hkn]
        rw [hd, hm, Nat.div.go.eq_def]
        dsimp only
        rw [dif_neg hkn, Nat.mul_zero, Nat.zero_add]

/-- The quotient by a positive `k` is pinned by `k * q ≤ n < k * q + k`. -/
theorem div_bounds (n k : Nat) (hk : 0 < k) : k * (n / k) ≤ n ∧ n < k * (n / k) + k := by
  have e := div_add_mod n k hk
  have hr : n % k < k := Nat.mod_lt n hk
  constructor
  · have : k * (n / k) ≤ k * (n / k) + n % k := Nat.le_add_right _ _
    rw [e] at this
    exact this
  · have : k * (n / k) + n % k < k * (n / k) + k := Nat.add_lt_add_left hr _
    rw [e] at this
    exact this

theorem length_append {α : Type} : ∀ (a b : List α), (a ++ b).length = a.length + b.length
  | [], b => (Nat.zero_add _).symm
  | _ :: a, b => by
      show (a ++ b).length + 1 = a.length + 1 + b.length
      rw [length_append a b, Nat.add_right_comm]

theorem append_nil {α : Type} : ∀ (a : List α), a ++ [] = a
  | [] => rfl
  | x :: a => congrArg (x :: ·) (append_nil a)

theorem length_take_of_le {α : Type} : ∀ (n : Nat) (l : List α), n ≤ l.length →
    (l.take n).length = n
  | 0, _, _ => rfl
  | _ + 1, [], h => absurd h (Nat.not_succ_le_zero _)
  | n + 1, _ :: l, h => congrArg (· + 1) (length_take_of_le n l (Nat.le_of_succ_le_succ h))

theorem take_append_of_le {α : Type} : ∀ (n : Nat) (a b : List α), n ≤ a.length →
    (a ++ b).take n = a.take n
  | 0, _, _, _ => rfl
  | _ + 1, [], _, h => absurd h (Nat.not_succ_le_zero _)
  | n + 1, x :: a, b, h =>
      congrArg (x :: ·) (take_append_of_le n a b (Nat.le_of_succ_le_succ h))

theorem drop_append_of_le {α : Type} : ∀ (n : Nat) (a b : List α), n ≤ a.length →
    (a ++ b).drop n = a.drop n ++ b
  | 0, _, _, _ => rfl
  | _ + 1, [], _, h => absurd h (Nat.not_succ_le_zero _)
  | n + 1, _ :: a, b, h => drop_append_of_le n a b (Nat.le_of_succ_le_succ h)

theorem drop_drop {α : Type} : ∀ (m n : Nat) (l : List α), (l.drop m).drop n = l.drop (m + n)
  | 0, n, l => by rw [Nat.zero_add]; rfl
  | _ + 1, n, [] => by cases n <;> rfl
  | m + 1, n, _ :: l => by
      show (l.drop m).drop n = (List.drop (m + 1 + n) (_ :: l))
      rw [Nat.add_right_comm]
      exact drop_drop m n l

/-- Equal sums with a common left summand have equal right summands. -/
theorem add_left_cancel' {n m k : Nat} (h : n + m = n + k) : m = k := by
  have h1 : m + n ≤ k + n := by rw [Nat.add_comm m n, Nat.add_comm k n]; exact Nat.le_of_eq h
  have h2 : k + n ≤ m + n := by rw [Nat.add_comm m n, Nat.add_comm k n]; exact Nat.le_of_eq h.symm
  exact Nat.le_antisymm (le_of_add_le_add_right n h1) (le_of_add_le_add_right n h2)

theorem sub_eq_of_eq_add {a b c : Nat} (h : a = b + c) : a - c = b := by
  rw [h]; exact add_sub_cancel b c

/-- The remainder is the unique `r < k` with `k * q + r = s`. -/
theorem mod_unique {s k q r : Nat} (hk : 0 < k) (h : k * q + r = s) (hr : r < k) : s % k = r := by
  have e := div_add_mod s k hk
  have hr' : s % k < k := Nat.mod_lt s hk
  have hq : q = s / k := by
    cases Nat.lt_or_ge q (s / k) with
    | inl hlt =>
        have h1 : k * q + r < k * (q + 1) := by rw [Nat.mul_succ]; exact Nat.add_lt_add_left hr _
        have h2 : k * (q + 1) ≤ k * (s / k) := Nat.mul_le_mul_left k hlt
        have h3 : k * (s / k) ≤ s := (div_bounds s k hk).1
        exact absurd (Nat.lt_of_lt_of_le h1 (Nat.le_trans h2 h3)) (by rw [h]; exact Nat.lt_irrefl s)
    | inr hge =>
        cases Nat.lt_or_ge (s / k) q with
        | inl hlt =>
            have h1 : s < k * (s / k + 1) := by rw [Nat.mul_succ]; exact (div_bounds s k hk).2
            have h2 : k * (s / k + 1) ≤ k * q := Nat.mul_le_mul_left k hlt
            have h3 : k * q ≤ s := by rw [← h]; exact Nat.le_add_right _ _
            exact absurd (Nat.lt_of_lt_of_le h1 (Nat.le_trans h2 h3)) (Nat.lt_irrefl s)
        | inr hge' => exact Nat.le_antisymm hge' hge
  rw [hq] at h
  exact (add_left_cancel' (h.trans e.symm)).symm

end Arith

open Arith

/-! ## Big-endian fields (7.5.8.2) -/

/-- The value of `bs` read high-order byte first, after the already-read prefix `acc`. -/
def beAcc : Nat → List Nat → Nat
  | acc, [] => acc
  | acc, b :: bs => beAcc (acc * 256 + b) bs

/-- A field's value: its bytes, high-order byte first. A zero-width field reads as 0. -/
def beNat (bs : List Nat) : Nat := beAcc 0 bs

/-- Every element is a byte. -/
def bytesOk : List Nat → Bool
  | [] => true
  | b :: bs => decide (b < 256) && bytesOk bs

theorem beAcc_lt : ∀ (bs : List Nat) (acc k : Nat), acc < 256 ^ k → bytesOk bs = true →
    beAcc acc bs < 256 ^ (k + bs.length)
  | [], _, _, h, _ => h
  | b :: bs, acc, k, h, hb => by
      have hb1 : b < 256 := of_decide_eq_true (band_left hb)
      have h1 : (acc + 1) * 256 ≤ 256 ^ k * 256 :=
        Nat.mul_le_mul_right 256 (Nat.succ_le_of_lt h)
      rw [Nat.succ_mul] at h1
      have hstep : acc * 256 + b < 256 ^ (k + 1) :=
        Nat.lt_of_lt_of_le (Nat.add_lt_add_left hb1 _) h1
      have ih := beAcc_lt bs (acc * 256 + b) (k + 1) hstep (band_right hb)
      have e : k + (b :: bs).length = k + 1 + bs.length := (Nat.add_right_comm k 1 bs.length).symm
      show beAcc (acc * 256 + b) bs < 256 ^ (k + (b :: bs).length)
      rw [e]
      exact ih

/-- A field of `w` bytes is below `256 ^ w`. -/
theorem beNat_lt (bs : List Nat) (h : bytesOk bs = true) : beNat bs < 256 ^ bs.length := by
  have := beAcc_lt bs 0 0 Nat.zero_lt_one h
  rw [Nat.zero_add] at this
  exact this

/-- Reading more bytes never lowers the value. -/
theorem beAcc_ge : ∀ (bs : List Nat) (acc : Nat), acc ≤ beAcc acc bs
  | [], _ => Nat.le_refl _
  | b :: bs, acc => Nat.le_trans
      (Nat.le_trans (Nat.le_mul_of_pos_right acc (by decide)) (Nat.le_add_right _ b))
      (beAcc_ge bs (acc * 256 + b))

/-- `xref_be_step`: append one byte to a field value; refuse a value of 64 bits or more. -/
def beStepFields (acc b : Nat) : Bool × Nat :=
  if acc * 256 + b < 2 ^ 64 then (true, acc * 256 + b) else (false, 0)

/-- The Rust loop over a field's bytes: stop with `none` at the first refused step. -/
def beFold : Nat → List Nat → Option Nat
  | acc, [] => some acc
  | acc, b :: bs =>
      match beStepFields acc b with
      | (true, v) => beFold v bs
      | (false, _) => none

/-- The checked loop computes the exact big-endian value, or refuses exactly when it does
    not fit in 64 bits; it never wraps. -/
theorem beFold_spec : ∀ (bs : List Nat) (acc : Nat), acc < 2 ^ 64 →
    beFold acc bs = if beAcc acc bs < 2 ^ 64 then some (beAcc acc bs) else none
  | [], _, h => (if_pos h).symm
  | b :: bs, acc, _ => by
      show beFold acc (b :: bs) =
        (if beAcc (acc * 256 + b) bs < 2 ^ 64 then some (beAcc (acc * 256 + b) bs) else none)
      by_cases hs : acc * 256 + b < 2 ^ 64
      · have e : beStepFields acc b = (true, acc * 256 + b) := if_pos hs
        show (match beStepFields acc b with
          | (true, v) => beFold v bs
          | (false, _) => none) = _
        rw [e]
        exact beFold_spec bs (acc * 256 + b) hs
      · have e : beStepFields acc b = (false, 0) := if_neg hs
        have hge : ¬ beAcc (acc * 256 + b) bs < 2 ^ 64 := fun hlt =>
          hs (Nat.lt_of_le_of_lt (beAcc_ge bs _) hlt)
        show (match beStepFields acc b with
          | (true, v) => beFold v bs
          | (false, _) => none) = _
        rw [e, if_neg hge]

theorem beFold_zero (bs : List Nat) :
    beFold 0 bs = if beNat bs < 2 ^ 64 then some (beNat bs) else none :=
  beFold_spec bs 0 (by decide)

/-- A field of at most 8 bytes is never refused. -/
theorem beFold_fits_8 (bs : List Nat) (h : bytesOk bs = true) (hlen : bs.length ≤ 8) :
    beFold 0 bs = some (beNat bs) := by
  have h1 : beNat bs < 256 ^ bs.length := beNat_lt bs h
  have h2 : 256 ^ bs.length ≤ 256 ^ 8 := Nat.pow_le_pow_right (by decide) hlen
  have h3 : (256 : Nat) ^ 8 = 2 ^ 64 := by decide
  rw [beFold_zero, if_pos (h3 ▸ Nat.lt_of_lt_of_le h1 h2)]

/-! ## Cross-reference stream rows (7.5.8.2, Tables 17 and 18) -/

/-- The `/W` array: the byte widths of the three fields of an entry. -/
structure Widths where
  w1 : Nat
  w2 : Nat
  w3 : Nat

/-- The bytes of one entry. -/
def rowLen (w : Widths) : Nat := w.w1 + w.w2 + w.w3

/-- `xref_row_len`: accept a `/W` whose row is non-empty and whose length fits in 64 bits. -/
def rowLenFields (w1 w2 w3 : Nat) : Bool × Nat :=
  if 0 < w1 + w2 + w3 ∧ w1 + w2 + w3 < 2 ^ 64 then (true, w1 + w2 + w3) else (false, 0)

theorem rowLenFields_spec (w : Widths) (h : 0 < rowLen w) (h64 : rowLen w < 2 ^ 64) :
    rowLenFields w.w1 w.w2 w.w3 = (true, rowLen w) :=
  if_pos ⟨h, h64⟩

/-- An empty row (`/W [0 0 0]`) is refused: `/Size` alone would otherwise set the number of
    entries. -/
theorem rowLenFields_zero_refused (w1 w2 w3 : Nat) (h : w1 + w2 + w3 = 0) :
    rowLenFields w1 w2 w3 = (false, 0) :=
  if_neg (fun hc => Nat.lt_irrefl 0 (by rw [h] at hc; exact hc.1))

/-- Row `i` of the decoded stream: the `len` bytes from `i * len`. -/
def rowAt (data : List Nat) (len i : Nat) : List Nat := (data.drop (i * len)).take len

/-- `row_start`: row `i` is present when it ends within the data; it starts at `i * len`. -/
def rowStartFields (i len dataLen : Nat) : Bool × Nat :=
  if (i + 1) * len ≤ dataLen then (true, i * len) else (false, 0)

theorem rowStart_iff (i len dataLen : Nat) :
    (rowStartFields i len dataLen).1 = true ↔ (i + 1) * len ≤ dataLen := by
  unfold rowStartFields
  by_cases h : (i + 1) * len ≤ dataLen
  · rw [if_pos h]; exact ⟨fun _ => h, fun _ => rfl⟩
  · rw [if_neg h]; exact ⟨fun hf => Bool.noConfusion hf, fun hh => absurd hh h⟩

theorem rowStart_within {i len dataLen s : Nat} (h : rowStartFields i len dataLen = (true, s)) :
    s = i * len ∧ s + len ≤ dataLen := by
  unfold rowStartFields at h
  by_cases hc : (i + 1) * len ≤ dataLen
  · rw [if_pos hc] at h
    have hs : i * len = s := congrArg Prod.snd h
    rw [← hs]
    refine ⟨rfl, ?_⟩
    rw [← Nat.succ_mul]
    exact hc
  · rw [if_neg hc] at h
    have hf : false = true := congrArg Prod.fst h
    exact Bool.noConfusion hf

theorem rowAt_length (data : List Nat) (len i : Nat) (h : (i + 1) * len ≤ data.length) :
    (rowAt data len i).length = len := by
  apply length_take_of_le
  rw [List.length_drop]
  apply le_sub_of_add_le
  rw [← Nat.succ_mul]
  exact h

/-- A present row is read from its own bytes only: nothing after it is looked at. -/
theorem rowAt_append (data extra : List Nat) (len i : Nat) (h : (i + 1) * len ≤ data.length) :
    rowAt (data ++ extra) len i = rowAt data len i := by
  have h' : i * len + len ≤ data.length := by rw [← Nat.succ_mul]; exact h
  have hi : i * len ≤ data.length := Nat.le_trans (Nat.le_add_right _ _) h'
  unfold rowAt
  rw [drop_append_of_le _ _ _ hi, take_append_of_le]
  rw [List.length_drop]
  exact le_sub_of_add_le h'

/-- Field 1 (the type) of a row. -/
def field1 (w : Widths) (row : List Nat) : Nat := beNat (row.take w.w1)
/-- Field 2 of a row. -/
def field2 (w : Widths) (row : List Nat) : Nat := beNat ((row.drop w.w1).take w.w2)
/-- Field 3 of a row. -/
def field3 (w : Widths) (row : List Nat) : Nat := beNat ((row.drop (w.w1 + w.w2)).take w.w3)

/-- Every byte the three fields read, at its absolute position in the decoded stream, lies
    inside the data when the row was accepted by `row_start`. -/
theorem field_bytes_within (w : Widths) {i dataLen s : Nat}
    (h : rowStartFields i (rowLen w) dataLen = (true, s)) :
    (∀ j, j < w.w1 → s + j < dataLen) ∧ (∀ j, j < w.w2 → s + w.w1 + j < dataLen) ∧
      (∀ j, j < w.w3 → s + (w.w1 + w.w2) + j < dataLen) := by
  have hrow : s + rowLen w ≤ dataLen := (rowStart_within h).2
  have e : rowLen w = w.w1 + w.w2 + w.w3 := rfl
  refine ⟨fun j hj => ?_, fun j hj => ?_, fun j hj => ?_⟩
  · apply Nat.lt_of_lt_of_le (Nat.add_lt_add_left hj s)
    apply Nat.le_trans _ hrow
    apply Nat.add_le_add_left
    rw [e, Nat.add_assoc]
    exact Nat.le_add_right _ _
  · apply Nat.lt_of_lt_of_le (Nat.add_lt_add_left hj (s + w.w1))
    apply Nat.le_trans _ hrow
    rw [e, Nat.add_assoc s w.w1 w.w2]
    exact Nat.add_le_add_left (Nat.le_add_right _ _) s
  · apply Nat.lt_of_lt_of_le (Nat.add_lt_add_left hj (s + (w.w1 + w.w2)))
    rw [e, ← Nat.add_assoc s] at hrow
    exact hrow

/-- A cross-reference entry (Table 18). -/
inductive XrefEntry where
  /-- Type 0: a free object (next free object number, generation). -/
  | free (next gen : Nat)
  /-- Type 1: an object in use at a byte offset, with its generation. -/
  | inUse (offset gen : Nat)
  /-- Type 2: object `index` of the object stream numbered `stream`. -/
  | compressed (stream index : Nat)
  /-- Any other type: a reference to the null object. -/
  | null
  deriving DecidableEq

/-- Table 18 on the three field values, by entry type. -/
def entryOfFields (type f2 f3 : Nat) : XrefEntry :=
  if type = 0 then .free f2 f3
  else if type = 1 then .inUse f2 f3
  else if type = 2 then .compressed f2 f3
  else .null

/-- Table 17: a missing type field (`W[0] = 0`) defaults to type 1. -/
def entryType (w : Widths) (row : List Nat) : Nat := if w.w1 = 0 then 1 else field1 w row

/-- The entry a row encodes. A zero-width field reads as 0, which is Table 18's default
    generation for type 1. -/
def decodeEntry (w : Widths) (row : List Nat) : XrefEntry :=
  entryOfFields (entryType w row) (field2 w row) (field3 w row)

/-- `xref_entry_type`: the type code, with every unknown type collapsed to 3. -/
def entryTypeCode (w1Zero : Bool) (f1 : Nat) : Nat :=
  if w1Zero = true then 1 else if f1 ≤ 2 then f1 else 3

theorem entryOfFields_ge3 {type : Nat} (f2 f3 : Nat) (h : 3 ≤ type) :
    entryOfFields type f2 f3 = .null := by
  have h0 : ¬ type = 0 := fun e => absurd (e ▸ h) (by decide)
  have h1 : ¬ type = 1 := fun e => absurd (e ▸ h) (by decide)
  have h2 : ¬ type = 2 := fun e => absurd (e ▸ h) (by decide)
  unfold entryOfFields
  rw [if_neg h0, if_neg h1, if_neg h2]

/-- The kernel's type code selects the same entry as Table 18. -/
theorem decodeEntry_code (w : Widths) (row : List Nat) :
    entryOfFields (entryTypeCode (decide (w.w1 = 0)) (field1 w row)) (field2 w row) (field3 w row)
      = decodeEntry w row := by
  unfold decodeEntry entryType entryTypeCode
  by_cases hw : w.w1 = 0
  · rw [if_pos (decide_eq_true hw), if_pos hw]
  · have hd : ¬ decide (w.w1 = 0) = true := fun hc => hw (of_decide_eq_true hc)
    rw [if_neg hd, if_neg hw]
    by_cases hf : field1 w row ≤ 2
    · rw [if_pos hf]
    · rw [if_neg hf, entryOfFields_ge3 _ _ (by decide),
        entryOfFields_ge3 _ _ (Nat.succ_le_of_lt (Nat.lt_of_not_le hf))]

/-- With no type field every entry is in use (Table 17's default). -/
theorem decode_w1_zero_in_use (w : Widths) (row : List Nat) (h : w.w1 = 0) :
    decodeEntry w row = .inUse (field2 w row) (field3 w row) := by
  unfold decodeEntry entryType entryOfFields
  rw [if_pos h, if_neg (by decide), if_pos rfl]

/-- An unknown type is the null object, never an object in use (Table 18). -/
theorem decode_unknown_null (w : Widths) (row : List Nat) (hw : w.w1 ≠ 0)
    (h : 3 ≤ field1 w row) : decodeEntry w row = .null := by
  unfold decodeEntry entryType
  rw [if_neg hw]
  exact entryOfFields_ge3 _ _ h

/-- A missing third field reads as generation 0. -/
theorem decode_gen_default (w : Widths) (row : List Nat) (h : w.w3 = 0) : field3 w row = 0 := by
  unfold field3
  rw [h]
  rfl

/-- Bytes after a complete row never change its entry. -/
theorem entryOfRow_append (w : Widths) (row extra : List Nat) (h : row.length = rowLen w) :
    decodeEntry w (row ++ extra) = decodeEntry w row := by
  have hl : w.w1 + w.w2 + w.w3 = row.length := h.symm
  have h1 : w.w1 ≤ row.length := by
    rw [← hl, Nat.add_assoc]; exact Nat.le_add_right _ _
  have h12 : w.w1 + w.w2 ≤ row.length := by rw [← hl]; exact Nat.le_add_right _ _
  have e1 : field1 w (row ++ extra) = field1 w row := by
    unfold field1; rw [take_append_of_le _ _ _ h1]
  have e2 : field2 w (row ++ extra) = field2 w row := by
    unfold field2
    rw [drop_append_of_le _ _ _ h1, take_append_of_le]
    rw [List.length_drop]
    apply le_sub_of_add_le
    exact h12
  have e3 : field3 w (row ++ extra) = field3 w row := by
    unfold field3
    rw [drop_append_of_le _ _ _ h12, take_append_of_le]
    rw [List.length_drop]
    apply le_sub_of_add_le
    rw [hl]
    exact Nat.le_refl _
  unfold decodeEntry entryType
  rw [e1, e2, e3]

/-! ### Subsections (`/Index`) and the bound on entries -/

/-- One `/Index` subsection: `count` entries numbered from `num`, read from consecutive rows
    starting at `row`; reading stops at the first row that is not complete in the data. -/
def subsection (w : Widths) (data : List Nat) : Nat → Nat → Nat → List (Nat × XrefEntry)
  | _, _, 0 => []
  | row, num, c + 1 =>
      if (row + 1) * rowLen w ≤ data.length then
        (num, decodeEntry w (rowAt data (rowLen w) row)) :: subsection w data (row + 1) (num + 1) c
      else []

/-- The subsections of `/Index`, `(first, count)` pairs, in order; rows run on across them. -/
def sectionFrom (w : Widths) (data : List Nat) : Nat → List (Nat × Nat) → List (Nat × XrefEntry)
  | _, [] => []
  | row, (first, count) :: rest =>
      subsection w data row first count ++ sectionFrom w data (row + count) rest

/-- The entries of a cross-reference stream with decoded data `data`. -/
def sectionEntries (w : Widths) (data : List Nat) (index : List (Nat × Nat)) :
    List (Nat × XrefEntry) :=
  sectionFrom w data 0 index

/-- Table 17: `/Index` defaults to `[0 Size]`. -/
def defaultIndex (size : Nat) : List (Nat × Nat) := [(0, size)]

theorem subsection_length_le (w : Widths) (data : List Nat) : ∀ (c row num : Nat),
    (subsection w data row num c).length ≤ c
  | 0, _, _ => Nat.le_refl 0
  | c + 1, row, num => by
      rw [subsection]
      by_cases h : (row + 1) * rowLen w ≤ data.length
      · rw [if_pos h]
        exact Nat.succ_le_succ (subsection_length_le w data c (row + 1) (num + 1))
      · rw [if_neg h]
        exact Nat.zero_le _

/-- The rows a subsection consumed all lie in the data. -/
theorem subsection_rows (w : Widths) (data : List Nat) : ∀ (c row num : Nat),
    (subsection w data row num c).length = 0 ∨
      (row + (subsection w data row num c).length) * rowLen w ≤ data.length
  | 0, _, _ => Or.inl rfl
  | c + 1, row, num => by
      rw [subsection]
      by_cases h : (row + 1) * rowLen w ≤ data.length
      · rw [if_pos h]
        refine Or.inr ?_
        show (row + ((subsection w data (row + 1) (num + 1) c).length + 1)) * rowLen w ≤ _
        cases subsection_rows w data c (row + 1) (num + 1) with
        | inl h0 => rw [h0]; exact h
        | inr hle =>
            rw [show row + ((subsection w data (row + 1) (num + 1) c).length + 1)
                = row + 1 + (subsection w data (row + 1) (num + 1) c).length from
                (Nat.add_right_comm row 1 _).symm]
            exact hle
      · rw [if_neg h]
        exact Or.inl rfl

theorem sectionFrom_rows (w : Widths) (data : List Nat) : ∀ (index : List (Nat × Nat)) (row : Nat),
    (sectionFrom w data row index).length = 0 ∨
      (row + (sectionFrom w data row index).length) * rowLen w ≤ data.length
  | [], _ => Or.inl rfl
  | (first, count) :: rest, row => by
      show (subsection w data row first count ++ sectionFrom w data (row + count) rest).length = 0 ∨
        (row + (subsection w data row first count ++
          sectionFrom w data (row + count) rest).length) * rowLen w ≤ data.length
      rw [length_append]
      have hl1 := subsection_length_le w data count row first
      cases subsection_rows w data count row first with
      | inl h1 =>
          rw [h1, Nat.zero_add]
          cases sectionFrom_rows w data rest (row + count) with
          | inl h2 => exact Or.inl h2
          | inr h2 =>
              refine Or.inr (Nat.le_trans (Nat.mul_le_mul_right _ ?_) h2)
              exact Nat.add_le_add_right (Nat.le_add_right row count) _
      | inr h1 =>
          refine Or.inr ?_
          cases sectionFrom_rows w data rest (row + count) with
          | inl h2 => rw [h2, Nat.add_zero]; exact h1
          | inr h2 =>
              refine Nat.le_trans (Nat.mul_le_mul_right _ ?_) h2
              rw [← Nat.add_assoc]
              exact Nat.add_le_add_right (Nat.add_le_add_left hl1 row) _

/-- Every decoded entry used a complete row of the data. -/
theorem sectionEntries_rows (w : Widths) (data : List Nat) (index : List (Nat × Nat)) :
    (sectionEntries w data index).length * rowLen w ≤ data.length := by
  cases sectionFrom_rows w data index 0 with
  | inl h => unfold sectionEntries; rw [h, Nat.zero_mul]; exact Nat.zero_le _
  | inr h => unfold sectionEntries; rw [Nat.zero_add] at h; exact h

/-- With a non-empty row, the number of entries is bounded by the decoded data, whatever
    `/Size` and `/Index` claim. -/
theorem sectionEntries_bounded_by_data (w : Widths) (data : List Nat) (index : List (Nat × Nat))
    (h : 0 < rowLen w) : (sectionEntries w data index).length ≤ data.length :=
  Nat.le_trans (Nat.le_mul_of_pos_right _ h) (sectionEntries_rows w data index)

/-! ## Sections, incremental updates and resolution (7.3.10, 7.5.6, 7.5.7, 7.5.8.3) -/

/-- The entry a section gives object `num`, if it lists it. -/
def findEntry (num : Nat) : List (Nat × XrefEntry) → Option XrefEntry
  | [] => none
  | (n, e) :: rest => if n = num then some e else findEntry num rest

/-- The governing entry of `num`: sections newest first (the section at `startxref`, then
    its `/Prev` chain; for a hybrid file each revision's table, then its `/XRefStm`). The
    newest section that lists the object decides, a free entry included. -/
def lookup : List (List (Nat × XrefEntry)) → Nat → XrefEntry
  | [], _ => .null
  | s :: older, num =>
      match findEntry num s with
      | some e => e
      | none => lookup older num

theorem lookup_empty (num : Nat) : lookup [] num = .null := rfl

theorem lookup_newest {s : List (Nat × XrefEntry)} {num : Nat} {e : XrefEntry}
    (older : List (List (Nat × XrefEntry))) (h : findEntry num s = some e) :
    lookup (s :: older) num = e := by
  show (match findEntry num s with
    | some e => e
    | none => lookup older num) = e
  rw [h]

theorem lookup_older {s : List (Nat × XrefEntry)} {num : Nat}
    (older : List (List (Nat × XrefEntry))) (h : findEntry num s = none) :
    lookup (s :: older) num = lookup older num := by
  show (match findEntry num s with
    | some e => e
    | none => lookup older num) = _
  rw [h]

/-- Where an object is read from. -/
inductive Location where
  /-- Nothing: the reference is to the null object (7.3.10). -/
  | absent
  /-- An indirect object at this byte offset of the file. -/
  | direct (offset : Nat)
  /-- Object `index` of the object stream numbered `stream`. -/
  | inStream (stream index : Nat)
  deriving DecidableEq

/-- The location is an in-file offset. -/
def isDirect : Location → Bool
  | .direct _ => true
  | .absent => false
  | .inStream _ _ => false

/-- `xref_offset_ok`: a byte offset names a position inside the file. -/
def offsetOk (offset fileLen : Nat) : Bool := decide (offset < fileLen)

theorem offsetOk_iff (offset fileLen : Nat) : offsetOk offset fileLen = true ↔ offset < fileLen :=
  ⟨of_decide_eq_true, decide_eq_true⟩

/-- What one entry allows for a reference with generation `gen`. -/
def entryLocation (fileLen gen : Nat) : XrefEntry → Location
  | .inUse off g => if offsetOk off fileLen = true ∧ g = gen then .direct off else .absent
  | .compressed s i => if gen = 0 then .inStream s i else .absent
  | .free _ _ => .absent
  | .null => .absent

/-- `objstm_ref_ok`: a compressed object is read only from an object stream that is itself
    an in-file object in use with generation 0 (`stream_direct`), is not the object itself,
    has an accepted header (`has_n`), and has more than `index` objects. -/
def objstmRefOk (streamDirect isSelf hasN : Bool) (n index : Nat) : Bool :=
  streamDirect && !isSelf && hasN && decide (index < n)

theorem objstmRefOk_iff (streamDirect isSelf hasN : Bool) (n index : Nat) :
    objstmRefOk streamDirect isSelf hasN n index = true ↔
      streamDirect = true ∧ isSelf = false ∧ hasN = true ∧ index < n := by
  cases streamDirect <;> cases isSelf <;> cases hasN <;>
    first
    | exact ⟨fun h => ⟨rfl, rfl, rfl, of_decide_eq_true h⟩, fun h => decide_eq_true h.2.2.2⟩
    | exact ⟨fun h => Bool.noConfusion h, fun h => Bool.noConfusion h.1⟩
    | exact ⟨fun h => Bool.noConfusion h, fun h => Bool.noConfusion h.2.1⟩
    | exact ⟨fun h => Bool.noConfusion h, fun h => Bool.noConfusion h.2.2.1⟩

/-- Resolve the reference `num gen R`, given the governing entry of every object and the
    `/N` of every object stream whose header was accepted. -/
def resolve (fileLen : Nat) (entryOf : Nat → XrefEntry) (streamN : Nat → Option Nat)
    (num gen : Nat) : Location :=
  match entryLocation fileLen gen (entryOf num) with
  | .inStream s i =>
      if objstmRefOk (isDirect (entryLocation fileLen 0 (entryOf s))) (decide (s = num))
          (streamN s).isSome ((streamN s).getD 0) i = true
      then .inStream s i else .absent
  | .direct off => .direct off
  | .absent => .absent

theorem entryLocation_direct {fileLen gen off : Nat} :
    ∀ {e : XrefEntry}, entryLocation fileLen gen e = .direct off →
      e = .inUse off gen ∧ off < fileLen
  | .inUse o g, h => by
      unfold entryLocation at h
      dsimp only at h
      by_cases hc : offsetOk o fileLen = true ∧ g = gen
      · rw [if_pos hc] at h
        injection h with ho
        rw [← ho, hc.2]
        exact ⟨rfl, (offsetOk_iff o fileLen).mp hc.1⟩
      · rw [if_neg hc] at h
        exact Location.noConfusion h
  | .compressed s i, h => by
      unfold entryLocation at h
      dsimp only at h
      by_cases hc : gen = 0
      · rw [if_pos hc] at h; exact Location.noConfusion h
      · rw [if_neg hc] at h; exact Location.noConfusion h
  | .free _ _, h => Location.noConfusion h
  | .null, h => Location.noConfusion h

theorem entryLocation_inStream {fileLen gen s i : Nat} :
    ∀ {e : XrefEntry}, entryLocation fileLen gen e = .inStream s i →
      e = .compressed s i ∧ gen = 0
  | .inUse o g, h => by
      unfold entryLocation at h
      dsimp only at h
      by_cases hc : offsetOk o fileLen = true ∧ g = gen
      · rw [if_pos hc] at h; exact Location.noConfusion h
      · rw [if_neg hc] at h; exact Location.noConfusion h
  | .compressed s' i', h => by
      unfold entryLocation at h
      dsimp only at h
      by_cases hc : gen = 0
      · rw [if_pos hc] at h
        injection h with hs hi
        rw [hs, hi]
        exact ⟨rfl, hc⟩
      · rw [if_neg hc] at h; exact Location.noConfusion h
  | .free _ _, h => Location.noConfusion h
  | .null, h => Location.noConfusion h

theorem isDirect_true : ∀ {l : Location}, isDirect l = true → ∃ off, l = .direct off
  | .direct off, _ => ⟨off, rfl⟩
  | .absent, h => Bool.noConfusion h
  | .inStream _ _, h => Bool.noConfusion h

/-- An in-use object is read only at an offset inside the file. -/
theorem resolve_direct_in_file {fileLen : Nat} {entryOf : Nat → XrefEntry}
    {streamN : Nat → Option Nat} {num gen off : Nat}
    (h : resolve fileLen entryOf streamN num gen = .direct off) : off < fileLen := by
  unfold resolve at h
  cases hl : entryLocation fileLen gen (entryOf num) with
  | absent => rw [hl] at h; exact Location.noConfusion h
  | direct o =>
      rw [hl] at h
      injection h with ho
      rw [← ho]
      exact (entryLocation_direct hl).2
  | inStream s i =>
      rw [hl] at h
      dsimp only at h
      by_cases hc : objstmRefOk (isDirect (entryLocation fileLen 0 (entryOf s))) (decide (s = num))
          (streamN s).isSome ((streamN s).getD 0) i = true
      · rw [if_pos hc] at h; exact Location.noConfusion h
      · rw [if_neg hc] at h; exact Location.noConfusion h

/-- A direct read is governed by an in-use entry of the reference's own generation. -/
theorem resolve_direct_entry {fileLen : Nat} {entryOf : Nat → XrefEntry}
    {streamN : Nat → Option Nat} {num gen off : Nat}
    (h : resolve fileLen entryOf streamN num gen = .direct off) : entryOf num = .inUse off gen := by
  unfold resolve at h
  cases hl : entryLocation fileLen gen (entryOf num) with
  | absent => rw [hl] at h; exact Location.noConfusion h
  | direct o =>
      rw [hl] at h
      injection h with ho
      rw [← ho]
      exact (entryLocation_direct hl).1
  | inStream s i =>
      rw [hl] at h
      dsimp only at h
      by_cases hc : objstmRefOk (isDirect (entryLocation fileLen 0 (entryOf s))) (decide (s = num))
          (streamN s).isSome ((streamN s).getD 0) i = true
      · rw [if_pos hc] at h; exact Location.noConfusion h
      · rw [if_neg hc] at h; exact Location.noConfusion h

/-- Everything a compressed read requires (7.5.7, Table 18). -/
theorem resolve_in_stream {fileLen : Nat} {entryOf : Nat → XrefEntry}
    {streamN : Nat → Option Nat} {num gen s i : Nat}
    (h : resolve fileLen entryOf streamN num gen = .inStream s i) :
    entryOf num = .compressed s i ∧ gen = 0 ∧ s ≠ num ∧
      (∃ off, entryOf s = .inUse off 0 ∧ off < fileLen) ∧
      (∃ n, streamN s = some n ∧ i < n) := by
  unfold resolve at h
  cases hl : entryLocation fileLen gen (entryOf num) with
  | absent => rw [hl] at h; exact Location.noConfusion h
  | direct o => rw [hl] at h; exact Location.noConfusion h
  | inStream s' i' =>
      rw [hl] at h
      dsimp only at h
      by_cases hc : objstmRefOk (isDirect (entryLocation fileLen 0 (entryOf s'))) (decide (s' = num))
          (streamN s').isSome ((streamN s').getD 0) i' = true
      · rw [if_pos hc] at h
        injection h with hs hi
        rw [hs, hi] at hl hc
        have ⟨hd, hself, hn, hlt⟩ := (objstmRefOk_iff _ _ _ _ _).mp hc
        have ⟨e1, e2⟩ := entryLocation_inStream hl
        have ⟨off, hoff⟩ := isDirect_true hd
        have ⟨e3, e4⟩ := entryLocation_direct hoff
        refine ⟨e1, e2, fun hsn => ?_, ⟨off, e3, e4⟩, ?_⟩
        · rw [decide_eq_true hsn] at hself; exact Bool.noConfusion hself
        · cases hsn : streamN s with
          | none => rw [hsn] at hn; exact Bool.noConfusion hn
          | some n => rw [hsn] at hlt; exact ⟨n, rfl, hlt⟩
      · rw [if_neg hc] at h; exact Location.noConfusion h

/-- An object stream stored in an object stream never yields an object (7.5.7). -/
theorem resolve_no_nested_objstm {fileLen : Nat} {entryOf : Nat → XrefEntry}
    {streamN : Nat → Option Nat} {num gen s i s' i' : Nat}
    (hs : entryOf s = .compressed s' i') : resolve fileLen entryOf streamN num gen ≠ .inStream s i :=
  fun h => by
    have ⟨_, _, _, ⟨off, hoff, _⟩, _⟩ := resolve_in_stream h
    rw [hs] at hoff
    exact XrefEntry.noConfusion hoff

/-- A compressed read never uses an index at or past the stream's `/N`. -/
theorem resolve_index_below_n {fileLen : Nat} {entryOf : Nat → XrefEntry}
    {streamN : Nat → Option Nat} {num gen s i n : Nat}
    (h : resolve fileLen entryOf streamN num gen = .inStream s i) (hn : streamN s = some n) :
    i < n := by
  have ⟨_, _, _, _, ⟨n', hn', hlt⟩⟩ := resolve_in_stream h
  rw [hn] at hn'
  injection hn' with e
  rw [e]
  exact hlt

/-- A free (deleted) object reads as null. -/
theorem resolve_free_absent (fileLen : Nat) (entryOf : Nat → XrefEntry)
    (streamN : Nat → Option Nat) (num gen next g : Nat) (h : entryOf num = .free next g) :
    resolve fileLen entryOf streamN num gen = .absent := by
  unfold resolve
  rw [h]
  rfl

/-- An entry of unknown type reads as null. -/
theorem resolve_null_absent (fileLen : Nat) (entryOf : Nat → XrefEntry)
    (streamN : Nat → Option Nat) (num gen : Nat) (h : entryOf num = .null) :
    resolve fileLen entryOf streamN num gen = .absent := by
  unfold resolve
  rw [h]
  rfl

/-- A compressed object has generation 0; any other generation reads as null. -/
theorem resolve_compressed_gen (fileLen : Nat) (entryOf : Nat → XrefEntry)
    (streamN : Nat → Option Nat) (num gen s i : Nat) (h : entryOf num = .compressed s i)
    (hg : gen ≠ 0) : resolve fileLen entryOf streamN num gen = .absent := by
  unfold resolve
  rw [h]
  show (match (if gen = 0 then Location.inStream s i else Location.absent) with
    | .inStream s i =>
        if objstmRefOk (isDirect (entryLocation fileLen 0 (entryOf s))) (decide (s = num))
            (streamN s).isSome ((streamN s).getD 0) i = true
        then Location.inStream s i else Location.absent
    | .direct off => .direct off
    | .absent => .absent) = .absent
  rw [if_neg hg]

/-- An object deleted in a newer revision stays deleted, whatever older revisions say. -/
theorem lookup_deleted_stays_deleted (fileLen : Nat) (streamN : Nat → Option Nat)
    (s : List (Nat × XrefEntry)) (older : List (List (Nat × XrefEntry))) (num gen next g : Nat)
    (h : findEntry num s = some (.free next g)) :
    resolve fileLen (lookup (s :: older)) streamN num gen = .absent :=
  resolve_free_absent fileLen _ streamN num gen next g (lookup_newest older h)

/-! ## Object streams (7.5.7) -/

/-- A decoded object stream: its length, `/First`, and the header's `(object number,
    offset)` pairs (offsets relative to `/First`). -/
structure ObjStm where
  len : Nat
  first : Nat
  pairs : List (Nat × Nat)

/-- The offsets are strictly increasing. -/
def increasing : List (Nat × Nat) → Bool
  | [] => true
  | [_] => true
  | (_, o1) :: (n2, o2) :: rest => decide (o1 < o2) && increasing ((n2, o2) :: rest)

/-- Every object starts inside the stream: `first + offset < len`. -/
def allIn (first len : Nat) : List (Nat × Nat) → Bool
  | [] => true
  | (_, o) :: rest => decide (first + o < len) && allIn first len rest

/-- `objstm_header_ok`: `/First` lies in the stream and the header can hold `N` pairs
    (`2N` integers of at least one digit separated by white space: `4N - 1 ≤ First`). -/
def objstmHeaderFields (first len n : Nat) : Bool :=
  decide (first ≤ len) && decide (4 * n ≤ first + 1)

/-- An accepted header: the kernel check, exactly `N` pairs, increasing offsets, and every
    object starting inside the stream. -/
def headerOk (m : ObjStm) (n : Nat) : Bool :=
  objstmHeaderFields m.first m.len n && decide (m.pairs.length = n) && increasing m.pairs &&
    allIn m.first m.len m.pairs

theorem none_some {α : Type} {β : Prop} {x : α} (h : (none : Option α) = some x) : β :=
  nomatch h

/-- The `i`-th element, if any. -/
def nth? {α : Type} : List α → Nat → Option α
  | [], _ => none
  | x :: _, 0 => some x
  | _ :: l, i + 1 => nth? l i

/-- Whether object `i` has a successor in the header. -/
def hasNext (m : ObjStm) (i : Nat) : Bool := (nth? m.pairs (i + 1)).isSome

/-- The successor's offset (0 when there is none). -/
def nextOff (m : ObjStm) (i : Nat) : Nat :=
  match nth? m.pairs (i + 1) with
  | some p => p.2
  | none => 0

/-- Object `i` ends where the next begins, the last at the end of the stream. -/
def spanEnd (m : ObjStm) (i : Nat) : Nat :=
  if hasNext m i = true then m.first + nextOff m i else m.len

/-- The byte extent `[start, end)` of object `i` in the decoded stream. -/
def objSpan (m : ObjStm) (i : Nat) : Option (Nat × Nat) :=
  match nth? m.pairs i with
  | some p => some (m.first + p.2, spanEnd m i)
  | none => none

/-- `objstm_span`: accept the extent of an object at `first + off` ending at the next
    object's start (or `len`) only when it is non-empty and inside the stream. -/
def spanFields (first off : Nat) (hasNext : Bool) (next len : Nat) : Bool × Nat × Nat :=
  if first + off < (if hasNext = true then first + next else len) ∧
      (if hasNext = true then first + next else len) ≤ len
  then (true, first + off, if hasNext = true then first + next else len)
  else (false, 0, 0)

/-- Whatever the header says, an accepted extent is non-empty, starts after the header
    (at or past `/First`) and ends inside the stream. -/
theorem spanFields_within {first off next len s e : Nat} {hn : Bool}
    (h : spanFields first off hn next len = (true, s, e)) :
    s = first + off ∧ first ≤ s ∧ s < e ∧ e ≤ len := by
  unfold spanFields at h
  by_cases hc : first + off < (if hn = true then first + next else len) ∧
      (if hn = true then first + next else len) ≤ len
  · rw [if_pos hc] at h
    injection h with _ h2
    injection h2 with hs he
    rw [← hs, ← he]
    exact ⟨rfl, Nat.le_add_right _ _, hc.1, hc.2⟩
  · rw [if_neg hc] at h
    have hf : false = true := congrArg Prod.fst h
    exact Bool.noConfusion hf

/-- The header check bounds `N` by `/First`, hence by the decoded data, before anything is
    allocated for it. -/
theorem objstmHeader_bounds_n {first len n : Nat} (h : objstmHeaderFields first len n = true) :
    n ≤ first ∧ first ≤ len := by
  have h1 : first ≤ len := of_decide_eq_true (band_left h)
  have h2 : 4 * n ≤ first + 1 := of_decide_eq_true (band_right h)
  refine ⟨?_, h1⟩
  cases n with
  | zero => exact Nat.zero_le _
  | succ k =>
      rw [Nat.mul_succ] at h2
      have h3 : 4 * k + 3 ≤ first := Nat.le_of_succ_le_succ h2
      have h4 : k ≤ 4 * k := Nat.le_mul_of_pos_left k (by decide)
      exact Nat.le_trans (Nat.add_le_add h4 (by decide : 1 ≤ 3)) h3

theorem headerOk_header_fields {m : ObjStm} {n : Nat} (h : headerOk m n = true) :
    objstmHeaderFields m.first m.len n = true :=
  band_left (band_left (band_left h))

theorem headerOk_length {m : ObjStm} {n : Nat} (h : headerOk m n = true) : m.pairs.length = n :=
  of_decide_eq_true (band_right (band_left (band_left h)))

theorem headerOk_increasing {m : ObjStm} {n : Nat} (h : headerOk m n = true) :
    increasing m.pairs = true :=
  band_right (band_left h)

theorem headerOk_allIn {m : ObjStm} {n : Nat} (h : headerOk m n = true) :
    allIn m.first m.len m.pairs = true :=
  band_right h

theorem nth?_lt {α : Type} : ∀ (l : List α) (i : Nat), i < l.length → ∃ x, nth? l i = some x
  | [], _, h => absurd h (Nat.not_lt_zero _)
  | x :: _, 0, _ => ⟨x, rfl⟩
  | _ :: l, i + 1, h => nth?_lt l i (Nat.lt_of_succ_lt_succ h)

theorem nth?_ge {α : Type} : ∀ (l : List α) (i : Nat), l.length ≤ i → nth? l i = none
  | [], _, _ => rfl
  | _ :: _, 0, h => absurd h (Nat.not_succ_le_zero _)
  | _ :: l, i + 1, h => nth?_ge l i (Nat.le_of_succ_le_succ h)

theorem nth?_some_lt {α : Type} : ∀ (l : List α) (i : Nat) (x : α), nth? l i = some x →
    i < l.length
  | [], _, _, h => none_some h
  | _ :: _, 0, _, _ => Nat.zero_lt_succ _
  | _ :: l, i + 1, x, h => Nat.succ_lt_succ (nth?_some_lt l i x h)

theorem allIn_nth {first len : Nat} : ∀ (l : List (Nat × Nat)) (i : Nat) (p : Nat × Nat),
    allIn first len l = true → nth? l i = some p → first + p.2 < len
  | [], _, _, _, h => none_some h
  | (o, x) :: _, 0, p, ha, h => by
      injection h with e
      rw [← e]
      exact of_decide_eq_true (band_left ha)
  | _ :: l, i + 1, p, ha, h => allIn_nth l i p (band_right ha) h

theorem increasing_tail : ∀ (q : Nat × Nat) (l : List (Nat × Nat)),
    increasing (q :: l) = true → increasing l = true
  | _, [], _ => rfl
  | (_, _), (_, _) :: _, h => band_right h

theorem increasing_head : ∀ (q r : Nat × Nat) (l : List (Nat × Nat)),
    increasing (q :: r :: l) = true → q.2 < r.2
  | (_, _), (_, _), _, h => of_decide_eq_true (band_left h)

/-- Increasing offsets: a later object never starts before an earlier one. -/
theorem increasing_mono : ∀ (l : List (Nat × Nat)) (a b : Nat) (pa pb : Nat × Nat),
    increasing l = true → a ≤ b → nth? l a = some pa → nth? l b = some pb → pa.2 ≤ pb.2
  | [], _, _, _, _, _, _, ha, _ => none_some ha
  | q :: l, 0, 0, pa, pb, _, _, ha, hb => by
      injection ha with ea
      injection hb with eb
      rw [← ea, ← eb]
      exact Nat.le_refl _
  | q :: l, 0, b + 1, pa, pb, hi, _, ha, hb => by
      injection ha with ea
      rw [← ea]
      cases l with
      | nil => exact none_some hb
      | cons r l' =>
          have h1 : q.2 < r.2 := increasing_head q r l' hi
          have h2 : r.2 ≤ pb.2 :=
            increasing_mono (r :: l') 0 b r pb (increasing_tail q _ hi) (Nat.zero_le b) rfl hb
          exact Nat.le_of_lt (Nat.lt_of_lt_of_le h1 h2)
  | _ :: _, _ + 1, 0, _, _, _, hab, _, _ => absurd hab (Nat.not_succ_le_zero _)
  | q :: l, a + 1, b + 1, pa, pb, hi, hab, ha, hb =>
      increasing_mono l a b pa pb (increasing_tail q l hi) (Nat.le_of_succ_le_succ hab) ha hb

/-- Strict version for adjacent objects. -/
theorem increasing_succ : ∀ (l : List (Nat × Nat)) (a : Nat) (pa pb : Nat × Nat),
    increasing l = true → nth? l a = some pa → nth? l (a + 1) = some pb → pa.2 < pb.2
  | [], _, _, _, _, ha, _ => none_some ha
  | [_], 0, _, _, _, _, hb => none_some hb
  | q :: r :: l, 0, pa, pb, hi, ha, hb => by
      injection ha with ea
      injection hb with eb
      rw [← ea, ← eb]
      exact increasing_head q r l hi
  | [_], _ + 1, _, _, _, ha, _ => none_some ha
  | q :: r :: l, a + 1, pa, pb, hi, ha, hb =>
      increasing_succ (r :: l) a pa pb (increasing_tail q _ hi) ha hb

/-- Under an accepted header, object `i` has an extent exactly when `i < N`. -/
theorem objSpan_some_iff {m : ObjStm} {n : Nat} (h : headerOk m n = true) (i : Nat) :
    (objSpan m i).isSome = true ↔ i < n := by
  rw [← headerOk_length h]
  constructor
  · intro hs
    unfold objSpan at hs
    cases hp : nth? m.pairs i with
    | none => rw [hp] at hs; exact Bool.noConfusion hs
    | some p => exact nth?_some_lt _ _ _ hp
  · intro hi
    have ⟨p, hp⟩ := nth?_lt _ _ hi
    unfold objSpan
    rw [hp]
    rfl

/-- An object starts at or after `/First`: no object overlaps the header. -/
theorem objSpan_after_header {m : ObjStm} {i s e : Nat} (h : objSpan m i = some (s, e)) :
    m.first ≤ s := by
  unfold objSpan at h
  cases hp : nth? m.pairs i with
  | none => rw [hp] at h; exact none_some h
  | some p =>
      rw [hp] at h
      injection h with h'
      injection h' with hs
      rw [← hs]
      exact Nat.le_add_right _ _

/-- Under an accepted header every extent is non-empty and inside `[First, len)`. -/
theorem objSpan_within {m : ObjStm} {n : Nat} (hh : headerOk m n = true) {i s e : Nat}
    (h : objSpan m i = some (s, e)) : m.first ≤ s ∧ s < e ∧ e ≤ m.len := by
  refine ⟨objSpan_after_header h, ?_⟩
  unfold objSpan at h
  cases hp : nth? m.pairs i with
  | none => rw [hp] at h; exact none_some h
  | some p =>
      rw [hp] at h
      injection h with h'
      injection h' with hs he
      rw [← hs, ← he]
      have hin : m.first + p.2 < m.len := allIn_nth _ _ _ (headerOk_allIn hh) hp
      unfold spanEnd hasNext nextOff
      cases hq : nth? m.pairs (i + 1) with
      | none => exact ⟨hin, Nat.le_refl _⟩
      | some q =>
          have hlt : p.2 < q.2 := increasing_succ _ _ _ _ (headerOk_increasing hh) hp hq
          have hqin : m.first + q.2 < m.len := allIn_nth _ _ _ (headerOk_allIn hh) hq
          exact ⟨Nat.add_lt_add_left hlt _, Nat.le_of_lt hqin⟩

/-- Extents are in header order and never overlap. -/
theorem objSpan_ordered {m : ObjStm} {n : Nat} (hh : headerOk m n = true) {i j s e s' e' : Nat}
    (hij : i < j) (hi : objSpan m i = some (s, e)) (hj : objSpan m j = some (s', e')) :
    e ≤ s' := by
  unfold objSpan at hi hj
  cases hpj : nth? m.pairs j with
  | none => rw [hpj] at hj; exact none_some hj
  | some pj =>
      rw [hpj] at hj
      injection hj with hj'
      injection hj' with hs' _
      rw [← hs']
      cases hpi : nth? m.pairs i with
      | none => rw [hpi] at hi; exact none_some hi
      | some pi =>
          rw [hpi] at hi
          injection hi with hi'
          injection hi' with _ he
          rw [← he]
          unfold spanEnd hasNext nextOff
          cases hq : nth? m.pairs (i + 1) with
          | none =>
              have hlen : m.pairs.length ≤ i + 1 := Nat.le_of_not_lt (fun hlt => by
                have ⟨x, hx⟩ := nth?_lt _ _ hlt
                rw [hq] at hx
                exact none_some hx)
              have hn : nth? m.pairs j = none := nth?_ge _ _ (Nat.le_trans hlen hij)
              rw [hpj] at hn
              exact none_some hn.symm
          | some q =>
              exact Nat.add_le_add_left
                (increasing_mono _ _ _ _ _ (headerOk_increasing hh) hij hq hpj) _

/-- Under an accepted header the kernel accepts every object's extent and computes it. -/
theorem objSpan_fields {m : ObjStm} {n : Nat} (hh : headerOk m n = true) {i s e : Nat}
    (h : objSpan m i = some (s, e)) :
    ∃ p, nth? m.pairs i = some p ∧
      spanFields m.first p.2 (hasNext m i) (nextOff m i) m.len = (true, s, e) := by
  have ⟨_, hse, hel⟩ := objSpan_within hh h
  unfold objSpan at h
  cases hp : nth? m.pairs i with
  | none => rw [hp] at h; exact none_some h
  | some p =>
      rw [hp] at h
      injection h with h'
      injection h' with hs he
      refine ⟨p, rfl, ?_⟩
      unfold spanFields
      have e1 : (if hasNext m i = true then m.first + nextOff m i else m.len) = e := he
      rw [e1, hs]
      rw [if_pos ⟨hse, hel⟩]

/-! ## The `/Prev` chain (7.5.5, 7.5.6, 7.5.8.4) -/

/-- Membership in a list of offsets. -/
def memB (x : Nat) : List Nat → Bool
  | [] => false
  | y :: l => decide (x = y) || memB x l

/-- `xref_prev_ok`: follow a `/Prev` (or `/XRefStm`, or `startxref`) offset only when it is
    inside the file and has not been read before. -/
def prevOkFields (offset fileLen : Nat) (seen : Bool) : Bool := decide (offset < fileLen) && !seen

theorem prevOkFields_iff (offset fileLen : Nat) (seen : Bool) :
    prevOkFields offset fileLen seen = true ↔ offset < fileLen ∧ seen = false := by
  cases seen
  · exact ⟨fun h => ⟨of_decide_eq_true (band_left h), rfl⟩,
      fun h => band_intro (decide_eq_true h.1) rfl⟩
  · exact ⟨fun h => Bool.noConfusion (band_right h), fun h => Bool.noConfusion h.2⟩

/-- The section offsets read so far, most recent first: each was accepted against all
    those read before it. -/
def chainOk (fileLen : Nat) : List Nat → Bool
  | [] => true
  | o :: older => prevOkFields o fileLen (memB o older) && chainOk fileLen older

/-- No section is read twice. -/
theorem chain_nodup {fileLen o : Nat} {older : List Nat} (h : chainOk fileLen (o :: older) = true) :
    memB o older = false :=
  ((prevOkFields_iff _ _ _).mp (band_left h)).2

theorem memB_cons {x y : Nat} {l : List Nat} (h : memB x (y :: l) = true) : x = y ∨ memB x l = true := by
  show x = y ∨ memB x l = true
  cases hd : decide (x = y) with
  | true => exact Or.inl (of_decide_eq_true hd)
  | false =>
      have : (decide (x = y) || memB x l) = true := h
      rw [hd] at this
      exact Or.inr this

theorem memB_self (x : Nat) (l : List Nat) : memB x (x :: l) = true := by
  show (decide (x = x) || memB x l) = true
  rw [decide_eq_true rfl]
  rfl

theorem memB_tail {x y : Nat} {l : List Nat} (h : memB x l = true) : memB x (y :: l) = true := by
  show (decide (x = y) || memB x l) = true
  rw [h]
  cases decide (x = y) <;> rfl

/-- Every section read is inside the file. -/
theorem chain_in_file {fileLen : Nat} : ∀ {offs : List Nat}, chainOk fileLen offs = true →
    ∀ x, memB x offs = true → x < fileLen
  | [], _, _, hx => Bool.noConfusion hx
  | o :: older, h, x, hx => by
      cases memB_cons hx with
      | inl e => rw [e]; exact ((prevOkFields_iff _ _ _).mp (band_left h)).1
      | inr hm => exact chain_in_file (band_right h) x hm

/-- The list without `v`. -/
def dropVal (v : Nat) : List Nat → List Nat
  | [] => []
  | x :: l => if x = v then dropVal v l else x :: dropVal v l

/-- No element repeats. -/
def nodupB : List Nat → Bool
  | [] => true
  | x :: l => !memB x l && nodupB l

theorem chain_nodupB {fileLen : Nat} : ∀ {offs : List Nat}, chainOk fileLen offs = true →
    nodupB offs = true
  | [], _ => rfl
  | _ :: _, h => band_intro (by rw [chain_nodup h]; rfl) (chain_nodupB (band_right h))

theorem memB_dropVal {v x : Nat} : ∀ {l : List Nat}, memB x (dropVal v l) = true →
    memB x l = true ∧ x ≠ v
  | [], h => Bool.noConfusion h
  | y :: l, h => by
      unfold dropVal at h
      by_cases hy : y = v
      · rw [if_pos hy] at h
        have ⟨h1, h2⟩ := memB_dropVal h
        exact ⟨memB_tail h1, h2⟩
      · rw [if_neg hy] at h
        cases memB_cons h with
        | inl e => exact ⟨by rw [e]; exact memB_self y l, fun ev => hy (e ▸ ev)⟩
        | inr hm =>
            have ⟨h1, h2⟩ := memB_dropVal hm
            exact ⟨memB_tail h1, h2⟩

theorem nodupB_dropVal (v : Nat) : ∀ {l : List Nat}, nodupB l = true → nodupB (dropVal v l) = true
  | [], _ => rfl
  | y :: l, h => by
      unfold dropVal
      by_cases hy : y = v
      · rw [if_pos hy]; exact nodupB_dropVal v (band_right h)
      · rw [if_neg hy]
        refine band_intro ?_ (nodupB_dropVal v (band_right h))
        cases hm : memB y (dropVal v l) with
        | false => rfl
        | true =>
            have h1 := (memB_dropVal hm).1
            have h2 : memB y l = false := bnot_true (band_left h)
            rw [h1] at h2
            exact Bool.noConfusion h2

theorem dropVal_absent {v : Nat} : ∀ {l : List Nat}, memB v l = false → dropVal v l = l
  | [], _ => rfl
  | y :: l, h => by
      unfold dropVal
      have hne : ¬ y = v := fun e => by
        rw [e, memB_self] at h
        exact Bool.noConfusion h
      rw [if_neg hne]
      have ht : memB v l = false := by
        cases hm : memB v l with
        | false => rfl
        | true => rw [memB_tail hm] at h; exact Bool.noConfusion h
      rw [dropVal_absent ht]

theorem length_dropVal (v : Nat) : ∀ {l : List Nat}, nodupB l = true →
    l.length ≤ (dropVal v l).length + 1
  | [], _ => Nat.zero_le _
  | y :: l, h => by
      unfold dropVal
      by_cases hy : y = v
      · rw [if_pos hy]
        have hv : memB v l = false := by rw [← hy]; exact bnot_true (band_left h)
        rw [dropVal_absent hv]
        exact Nat.le_refl _
      · rw [if_neg hy]
        exact Nat.succ_le_succ (length_dropVal v (band_right h))

/-- Pigeonhole: distinct numbers below `n` number at most `n`. -/
theorem nodup_below_length : ∀ (n : Nat) (l : List Nat), nodupB l = true →
    (∀ x, memB x l = true → x < n) → l.length ≤ n
  | 0, [], _, _ => Nat.le_refl 0
  | 0, x :: l, _, hb => absurd (hb x (memB_self x l)) (Nat.not_lt_zero x)
  | n + 1, l, hn, hb => by
      have hd : (dropVal n l).length ≤ n := by
        apply nodup_below_length n (dropVal n l) (nodupB_dropVal n hn)
        intro x hx
        have ⟨h1, h2⟩ := memB_dropVal hx
        exact Nat.lt_of_le_of_ne (Nat.le_of_lt_succ (hb x h1)) h2
      exact Nat.le_trans (length_dropVal n hn) (Nat.succ_le_succ hd)

/-- The `/Prev` chain cannot loop: it has at most `fileLen` sections. -/
theorem chain_length_le {fileLen : Nat} {offs : List Nat} (h : chainOk fileLen offs = true) :
    offs.length ≤ fileLen :=
  nodup_below_length fileLen offs (chain_nodupB h) (chain_in_file h)

/-! ## Stream extent (7.3.8.1) -/

/-- `stream_extent`: the `/Length` bytes from the data start must end inside the file. -/
def streamExtentFields (start length fileLen : Nat) : Bool × Nat :=
  if start + length ≤ fileLen then (true, start + length) else (false, 0)

theorem streamExtent_within {start length fileLen e : Nat}
    (h : streamExtentFields start length fileLen = (true, e)) :
    e = start + length ∧ start ≤ e ∧ e ≤ fileLen := by
  unfold streamExtentFields at h
  by_cases hc : start + length ≤ fileLen
  · rw [if_pos hc] at h
    injection h with _ he
    rw [← he]
    exact ⟨rfl, Nat.le_add_right _ _, hc⟩
  · rw [if_neg hc] at h
    have hf : false = true := congrArg Prod.fst h
    exact Bool.noConfusion hf

/-! ## Predictors (7.4.4.4, PNG 9) -/

/-- `predictor_kind`: `/Predictor` 1 none (0), 2 TIFF (1), 10 to 15 PNG (2); anything else
    is not defined by Table 8 and refuses the stream (3). -/
def predictorKind (p : Nat) : Nat :=
  if p = 1 then 0 else if p = 2 then 1 else if 10 ≤ p ∧ p ≤ 15 then 2 else 3

theorem predictorKind_cases (p : Nat) :
    (predictorKind p = 0 ↔ p = 1) ∧ (predictorKind p = 1 ↔ p = 2) ∧
      (predictorKind p = 2 ↔ 10 ≤ p ∧ p ≤ 15) := by
  unfold predictorKind
  by_cases h1 : p = 1
  · rw [if_pos h1]
    refine ⟨⟨fun _ => h1, fun _ => rfl⟩, ⟨fun h => absurd h (by decide), fun h => ?_⟩,
      ⟨fun h => absurd h (by decide), fun h => ?_⟩⟩
    · rw [h1] at h; exact absurd h (by decide)
    · rw [h1] at h; exact absurd h.1 (by decide)
  · rw [if_neg h1]
    by_cases h2 : p = 2
    · rw [if_pos h2]
      refine ⟨⟨fun h => absurd h (by decide), fun h => absurd h h1⟩, ⟨fun _ => h2, fun _ => rfl⟩,
        ⟨fun h => absurd h (by decide), fun h => ?_⟩⟩
      rw [h2] at h; exact absurd h.1 (by decide)
    · rw [if_neg h2]
      by_cases h3 : 10 ≤ p ∧ p ≤ 15
      · rw [if_pos h3]
        exact ⟨⟨fun h => absurd h (by decide), fun h => absurd h h1⟩,
          ⟨fun h => absurd h (by decide), fun h => absurd h h2⟩, ⟨fun _ => h3, fun _ => rfl⟩⟩
      · rw [if_neg h3]
        exact ⟨⟨fun h => absurd h (by decide), fun h => absurd h h1⟩,
          ⟨fun h => absurd h (by decide), fun h => absurd h h2⟩,
          ⟨fun h => absurd h (by decide), fun h => absurd h h3⟩⟩

/-- Table 8's `/BitsPerComponent` values. -/
def bpcOk (bpc : Nat) : Bool :=
  decide (bpc = 1) || decide (bpc = 2) || decide (bpc = 4) || decide (bpc = 8) || decide (bpc = 16)

/-- PNG `bpp`: bytes per complete pixel, rounded up. -/
def pixelBytes (colors bpc : Nat) : Nat := (colors * bpc + 7) / 8

/-- Bytes per row (without the tag byte): `Colors * BitsPerComponent * Columns` bits,
    rounded up to whole bytes. -/
def rowBytes (colors bpc columns : Nat) : Nat := (colors * bpc * columns + 7) / 8

/-- `png_geometry`: accept Table 8 parameters whose row size fits in 64 bits; give the row
    bytes and `bpp`. -/
def pngGeometryFields (colors bpc columns : Nat) : Bool × Nat × Nat :=
  if 1 ≤ colors ∧ bpcOk bpc = true ∧ 1 ≤ columns ∧ colors * bpc * columns + 7 < 2 ^ 64 then
    (true, rowBytes colors bpc columns, pixelBytes colors bpc)
  else (false, 0, 0)

/-- `(x + 7) / 8` is `x / 8` rounded up. -/
theorem ceil8 (x : Nat) : x ≤ 8 * ((x + 7) / 8) ∧ 8 * ((x + 7) / 8) < x + 8 := by
  have ⟨h1, h2⟩ := div_bounds (x + 7) 8 (by decide)
  refine ⟨?_, Nat.lt_of_le_of_lt h1 (Nat.add_lt_add_left (by decide) x)⟩
  apply le_of_add_le_add_right 8
  exact Nat.succ_le_of_lt h2

/-- A row holds every bit of its pixels and less than one byte of padding. -/
theorem rowBytes_bits (colors bpc columns : Nat) :
    colors * bpc * columns ≤ 8 * rowBytes colors bpc columns ∧
      8 * rowBytes colors bpc columns < colors * bpc * columns + 8 :=
  ceil8 _

theorem pixelBytes_pos {colors bpc : Nat} (hc : 1 ≤ colors) (hb : 1 ≤ bpc) :
    1 ≤ pixelBytes colors bpc := by
  have h1 : 1 ≤ colors * bpc := Nat.mul_le_mul hc hb
  have h2 := (ceil8 (colors * bpc)).1
  apply Nat.pos_of_ne_zero
  intro h0
  unfold pixelBytes at h0
  rw [h0] at h2
  exact absurd (Nat.le_trans h1 h2) (by decide)

/-- With the parameters a cross-reference stream uses (`/Colors 1 /BitsPerComponent 8
    /Columns` = the row length), a predictor row is exactly one entry. -/
theorem rowBytes_xref (len : Nat) : rowBytes 1 8 len = len := by
  have ⟨h1, h2⟩ := rowBytes_bits 1 8 len
  have e : 1 * 8 * len = 8 * len := by rw [Nat.one_mul]
  rw [e] at h1 h2
  apply Nat.le_antisymm
  · apply Nat.le_of_lt_succ
    apply Nat.lt_of_not_le
    intro hge
    have := Nat.mul_le_mul_left 8 hge
    rw [Nat.mul_succ] at this
    exact Nat.lt_irrefl _ (Nat.lt_of_lt_of_le h2 this)
  · exact Nat.le_of_mul_le_mul_left h1 (by decide)

/-- Sum of two bytes modulo 256. -/
def addByte (x y : Nat) : Nat := if x + y < 256 then x + y else x + y - 256

/-- Difference of two bytes modulo 256. -/
def subByte (x y : Nat) : Nat := if y ≤ x then x - y else x + 256 - y

theorem addByte_eq_mod {x y : Nat} (hx : x < 256) (hy : y < 256) : addByte x y = (x + y) % 256 := by
  unfold addByte
  by_cases h : x + y < 256
  · rw [if_pos h]
    exact (mod_unique (by decide) (by rw [Nat.mul_zero, Nat.zero_add]) h).symm
  · rw [if_neg h]
    have hge : 256 ≤ x + y := Nat.le_of_not_lt h
    have hlt : x + y < 256 + 256 := Nat.add_lt_add hx hy
    refine (mod_unique (q := 1) (by decide) ?_ (sub_lt_of_lt_add hge hlt)).symm
    rw [Nat.mul_one, Nat.add_comm]
    exact sub_add_cancel _ _ hge

theorem subByte_eq_mod {x y : Nat} (hx : x < 256) (hy : y < 256) :
    subByte x y = (x + 256 - y) % 256 := by
  unfold subByte
  by_cases h : y ≤ x
  · rw [if_pos h]
    have e : x + 256 - y = x - y + 256 := by
      apply sub_eq_of_eq_add
      rw [Nat.add_right_comm, sub_add_cancel x y h]
    rw [e]
    refine (mod_unique (q := 1) (by decide) ?_ (Nat.lt_of_le_of_lt (Nat.sub_le x y) hx)).symm
    rw [Nat.mul_one, Nat.add_comm]
  · rw [if_neg h]
    have hlt : x < y := Nat.lt_of_not_le h
    have hyx : y ≤ x + 256 := Nat.le_trans (Nat.le_of_lt hy) (Nat.le_add_left 256 x)
    have hs : x + 256 - y < 256 := by
      apply sub_lt_of_lt_add hyx
      rw [Nat.add_comm y, Nat.add_comm x]
      exact Nat.add_lt_add_left hlt 256
    exact (mod_unique (by decide) (by rw [Nat.mul_zero, Nat.zero_add]) hs).symm

theorem addByte_lt {x y : Nat} (hx : x < 256) (hy : y < 256) : addByte x y < 256 := by
  unfold addByte
  by_cases h : x + y < 256
  · rw [if_pos h]; exact h
  · rw [if_neg h]
    exact sub_lt_of_lt_add (Nat.le_of_not_lt h) (Nat.add_lt_add hx hy)

theorem addByte_subByte {x p : Nat} (hx : x < 256) (hp : p < 256) : addByte (subByte x p) p = x := by
  unfold subByte
  by_cases h : p ≤ x
  · rw [if_pos h]
    unfold addByte
    rw [sub_add_cancel x p h, if_pos hx]
  · rw [if_neg h]
    have hpx : p ≤ x + 256 := Nat.le_trans (Nat.le_of_lt hp) (Nat.le_add_left 256 x)
    unfold addByte
    rw [sub_add_cancel (x + 256) p hpx, if_neg (fun hc => absurd hc
      (Nat.not_lt_of_le (Nat.le_add_left 256 x)))]
    exact add_sub_cancel x 256

/-- `|x - y|`. -/
def absDiff (x y : Nat) : Nat := (x - y) + (y - x)

/-- PNG 9.4: with `p = a + b - c`, the nearest of `a`, `b`, `c` to `p`, ties in that order;
    `|p - a| = |b - c|`, `|p - b| = |a - c|`, `|p - c| = |a + b - 2c|`. -/
def paeth (a b c : Nat) : Nat :=
  if absDiff b c ≤ absDiff a c ∧ absDiff b c ≤ absDiff (a + b) (c + c) then a
  else if absDiff a c ≤ absDiff (a + b) (c + c) then b
  else c

theorem paeth_mem (a b c : Nat) : paeth a b c = a ∨ paeth a b c = b ∨ paeth a b c = c := by
  unfold paeth
  by_cases h1 : absDiff b c ≤ absDiff a c ∧ absDiff b c ≤ absDiff (a + b) (c + c)
  · rw [if_pos h1]; exact Or.inl rfl
  · rw [if_neg h1]
    by_cases h2 : absDiff a c ≤ absDiff (a + b) (c + c)
    · rw [if_pos h2]; exact Or.inr (Or.inl rfl)
    · rw [if_neg h2]; exact Or.inr (Or.inr rfl)

theorem paeth_lt {a b c : Nat} (ha : a < 256) (hb : b < 256) (hc : c < 256) : paeth a b c < 256 := by
  cases paeth_mem a b c with
  | inl e => rw [e]; exact ha
  | inr h => cases h with
    | inl e => rw [e]; exact hb
    | inr e => rw [e]; exact hc

/-- The PNG predictor of filter type `t` (PNG 9.2), `none` for an invalid type. -/
def predict (t a b c : Nat) : Option Nat :=
  if t = 0 then some 0
  else if t = 1 then some a
  else if t = 2 then some b
  else if t = 3 then some ((a + b) / 2)
  else if t = 4 then some (paeth a b c)
  else none

/-- Reconstruct one byte: the filtered byte plus the predictor, modulo 256. -/
def reconByte (t x a b c : Nat) : Option Nat := (predict t a b c).map (addByte x)

/-- Filter one byte (the encoder's side, used to state the round trip). -/
def filterByte (t x a b c : Nat) : Option Nat := (predict t a b c).map (subByte x)

/-- `png_recon_byte`. -/
def reconFields (t x a b c : Nat) : Bool × Nat :=
  match reconByte t x a b c with
  | some v => (true, v)
  | none => (false, 0)

theorem predict_none_iff (t a b c : Nat) : predict t a b c = none ↔ 5 ≤ t := by
  unfold predict
  by_cases h0 : t = 0
  · rw [if_pos h0, h0]; exact ⟨fun h => none_some h.symm, fun h => absurd h (by decide)⟩
  rw [if_neg h0]
  by_cases h1 : t = 1
  · rw [if_pos h1, h1]; exact ⟨fun h => none_some h.symm, fun h => absurd h (by decide)⟩
  rw [if_neg h1]
  by_cases h2 : t = 2
  · rw [if_pos h2, h2]; exact ⟨fun h => none_some h.symm, fun h => absurd h (by decide)⟩
  rw [if_neg h2]
  by_cases h3 : t = 3
  · rw [if_pos h3, h3]; exact ⟨fun h => none_some h.symm, fun h => absurd h (by decide)⟩
  rw [if_neg h3]
  by_cases h4 : t = 4
  · rw [if_pos h4, h4]; exact ⟨fun h => none_some h.symm, fun h => absurd h (by decide)⟩
  rw [if_neg h4]
  refine ⟨fun _ => ?_, fun _ => rfl⟩
  match t, h0, h1, h2, h3, h4 with
  | 0, h0, _, _, _, _ => exact absurd rfl h0
  | 1, _, h1, _, _, _ => exact absurd rfl h1
  | 2, _, _, h2, _, _ => exact absurd rfl h2
  | 3, _, _, _, h3, _ => exact absurd rfl h3
  | 4, _, _, _, _, h4 => exact absurd rfl h4
  | k + 5, _, _, _, _, _ => exact Nat.le_add_left 5 k

theorem predict_lt {t a b c p : Nat} (ha : a < 256) (hb : b < 256) (hc : c < 256)
    (h : predict t a b c = some p) : p < 256 := by
  unfold predict at h
  by_cases h0 : t = 0
  · rw [if_pos h0] at h; injection h with e; rw [← e]; decide
  rw [if_neg h0] at h
  by_cases h1 : t = 1
  · rw [if_pos h1] at h; injection h with e; rw [← e]; exact ha
  rw [if_neg h1] at h
  by_cases h2 : t = 2
  · rw [if_pos h2] at h; injection h with e; rw [← e]; exact hb
  rw [if_neg h2] at h
  by_cases h3 : t = 3
  · rw [if_pos h3] at h
    injection h with e
    rw [← e]
    have ⟨hq, _⟩ := div_bounds (a + b) 2 (by decide)
    apply Nat.lt_of_not_le
    intro hge
    have h512 : 2 * 256 ≤ a + b := Nat.le_trans (Nat.mul_le_mul_left 2 hge) hq
    exact absurd (Nat.lt_of_le_of_lt h512 (Nat.add_lt_add ha hb)) (by decide)
  rw [if_neg h3] at h
  by_cases h4 : t = 4
  · rw [if_pos h4] at h; injection h with e; rw [← e]; exact paeth_lt ha hb hc
  rw [if_neg h4] at h
  exact none_some h

/-- Reconstruction undoes filtering for every valid filter type (PNG 9.2). -/
theorem recon_filter_roundtrip {t x a b c y : Nat} (hx : x < 256) (ha : a < 256) (hb : b < 256)
    (hc : c < 256) (h : filterByte t x a b c = some y) : reconByte t y a b c = some x := by
  unfold filterByte at h
  unfold reconByte
  cases hp : predict t a b c with
  | none => rw [hp] at h; exact none_some h
  | some p =>
      rw [hp] at h
      injection h with e
      rw [← e]
      exact congrArg some (addByte_subByte hx (predict_lt ha hb hc hp))

/-- An invalid filter type is refused, never guessed. -/
theorem recon_invalid_tag {t : Nat} (x a b c : Nat) (h : 5 ≤ t) :
    reconByte t x a b c = none ∧ reconFields t x a b c = (false, 0) := by
  have hp : predict t a b c = none := (predict_none_iff t a b c).mpr h
  have hr : reconByte t x a b c = none := by unfold reconByte; rw [hp]; rfl
  refine ⟨hr, ?_⟩
  unfold reconFields
  rw [hr]

theorem recon_lt {t x a b c v : Nat} (hx : x < 256) (ha : a < 256) (hb : b < 256) (hc : c < 256)
    (h : reconByte t x a b c = some v) : v < 256 := by
  unfold reconByte at h
  cases hp : predict t a b c with
  | none => rw [hp] at h; exact none_some h
  | some p =>
      rw [hp] at h
      injection h with e
      rw [← e]
      exact addByte_lt hx (predict_lt ha hb hc hp)

theorem reconFields_spec (t x a b c v : Nat) :
    reconFields t x a b c = (true, v) ↔ reconByte t x a b c = some v := by
  unfold reconFields
  cases hr : reconByte t x a b c with
  | none =>
      exact ⟨fun h => by
        have hf : false = true := congrArg Prod.fst h
        exact Bool.noConfusion hf, fun h => none_some h⟩
  | some w =>
      refine ⟨fun h => ?_, fun h => ?_⟩
      · have e : w = v := congrArg Prod.snd h
        rw [e]
      · injection h with e
        rw [e]

/-- The `i`-th byte, 0 past the end (PNG: bytes outside the image are 0). -/
def nthD (l : List Nat) (i : Nat) : Nat :=
  match nth? l i with
  | some x => x
  | none => 0

/-- Reconstruct a row: `cur` is the part already reconstructed, `prev` the prior
    reconstructed row (empty for the first row: all zeros), `enc` the remaining filtered
    bytes. `a` is `bpp` bytes to the left (0 before the row), `b` above, `c` above `a`. -/
def reconRowGo (t bpp : Nat) (prev : List Nat) : List Nat → List Nat → Option (List Nat)
  | cur, [] => some cur
  | cur, x :: rest =>
      match reconByte t x (if bpp ≤ cur.length then nthD cur (cur.length - bpp) else 0)
          (nthD prev cur.length)
          (if bpp ≤ cur.length then nthD prev (cur.length - bpp) else 0) with
      | some v => reconRowGo t bpp prev (cur ++ [v]) rest
      | none => none

/-- Reconstruct one row with filter type `t`. -/
def reconRow (t bpp : Nat) (prev enc : List Nat) : Option (List Nat) := reconRowGo t bpp prev [] enc

theorem reconRowGo_length (t bpp : Nat) (prev : List Nat) : ∀ (enc cur out : List Nat),
    reconRowGo t bpp prev cur enc = some out → out.length = cur.length + enc.length
  | [], cur, out, h => by
      injection h with e
      rw [← e]
      rfl
  | x :: rest, cur, out, h => by
      unfold reconRowGo at h
      cases hv : reconByte t x (if bpp ≤ cur.length then nthD cur (cur.length - bpp) else 0)
          (nthD prev cur.length)
          (if bpp ≤ cur.length then nthD prev (cur.length - bpp) else 0) with
      | none => rw [hv] at h; exact none_some h
      | some v =>
          rw [hv] at h
          dsimp only at h
          rw [reconRowGo_length t bpp prev rest (cur ++ [v]) out h, length_append]
          show cur.length + 1 + rest.length = cur.length + (rest.length + 1)
          rw [Nat.add_right_comm, Nat.add_assoc]

/-- A reconstructed row has exactly as many bytes as the filtered row. -/
theorem reconRow_length {t bpp : Nat} {prev enc out : List Nat}
    (h : reconRow t bpp prev enc = some out) : out.length = enc.length := by
  rw [reconRowGo_length t bpp prev enc [] out h]
  exact Nat.zero_add _

theorem bytesOk_append : ∀ (a b : List Nat), bytesOk a = true → bytesOk b = true →
    bytesOk (a ++ b) = true
  | [], _, _, hb => hb
  | _ :: a, b, ha, hb => band_intro (band_left ha) (bytesOk_append a b (band_right ha) hb)

theorem nthD_lt : ∀ (l : List Nat) (i : Nat), bytesOk l = true → nthD l i < 256
  | [], _, _ => Nat.zero_lt_succ 255
  | _ :: _, 0, h => of_decide_eq_true (band_left h)
  | _ :: l, i + 1, h => nthD_lt l i (band_right h)

theorem ite_lt {c : Prop} [Decidable c] {x : Nat} (hx : x < 256) :
    (if c then x else 0) < 256 := by
  by_cases h : c
  · rw [if_pos h]; exact hx
  · rw [if_neg h]; decide

theorem reconRowGo_bytes (t bpp : Nat) (prev : List Nat) (hp : bytesOk prev = true) :
    ∀ (enc cur out : List Nat), bytesOk enc = true → bytesOk cur = true →
      reconRowGo t bpp prev cur enc = some out → bytesOk out = true
  | [], cur, out, _, hc, h => by
      injection h with e
      rw [← e]
      exact hc
  | x :: rest, cur, out, he, hc, h => by
      unfold reconRowGo at h
      cases hv : reconByte t x (if bpp ≤ cur.length then nthD cur (cur.length - bpp) else 0)
          (nthD prev cur.length)
          (if bpp ≤ cur.length then nthD prev (cur.length - bpp) else 0) with
      | none => rw [hv] at h; exact none_some h
      | some v =>
          rw [hv] at h
          dsimp only at h
          have hv' : v < 256 := recon_lt (of_decide_eq_true (band_left he))
            (ite_lt (nthD_lt cur _ hc)) (nthD_lt prev _ hp) (ite_lt (nthD_lt prev _ hp)) hv
          exact reconRowGo_bytes t bpp prev hp rest (cur ++ [v]) out (band_right he)
            (bytesOk_append cur [v] hc (band_intro (decide_eq_true hv') rfl)) h

/-- A reconstructed row is bytes. -/
theorem reconRow_bytes {t bpp : Nat} {prev enc out : List Nat} (hp : bytesOk prev = true)
    (he : bytesOk enc = true) (h : reconRow t bpp prev enc = some out) : bytesOk out = true :=
  reconRowGo_bytes t bpp prev hp enc [] out he rfl h

/-- Decode the rows of a predicted stream: each row is a tag byte and `rb` filtered bytes;
    a trailing partial row is not decoded; an invalid tag refuses the stream. `fuel` bounds
    the rows (each consumes at least one byte). -/
def pngDecodeGo (bpp rb : Nat) : Nat → List Nat → List Nat → Option (List Nat)
  | 0, _, _ => some []
  | _ + 1, _, [] => some []
  | fuel + 1, prev, tag :: rest =>
      if rb ≤ rest.length then
        match reconRow tag bpp prev (rest.take rb) with
        | some row =>
            match pngDecodeGo bpp rb fuel row (rest.drop rb) with
            | some out => some (row ++ out)
            | none => none
        | none => none
      else some []

/-- The PNG-predicted stream `data` (`/Predictor` 10 to 15). -/
def pngDecode (colors bpc columns : Nat) (data : List Nat) : Option (List Nat) :=
  pngDecodeGo (pixelBytes colors bpc) (rowBytes colors bpc columns) data.length [] data

theorem pngDecodeGo_rows (bpp rb : Nat) : ∀ (fuel : Nat) (prev data out : List Nat),
    pngDecodeGo bpp rb fuel prev data = some out →
      ∃ k, out.length = k * rb ∧ k * (rb + 1) ≤ data.length
  | 0, _, _, out, h => by
      injection h with e
      rw [← e]
      exact ⟨0, (Nat.zero_mul rb).symm, by rw [Nat.zero_mul]; exact Nat.zero_le _⟩
  | _ + 1, _, [], out, h => by
      injection h with e
      rw [← e]
      exact ⟨0, (Nat.zero_mul rb).symm, by rw [Nat.zero_mul]; exact Nat.zero_le _⟩
  | fuel + 1, prev, tag :: rest, out, h => by
      unfold pngDecodeGo at h
      by_cases hr : rb ≤ rest.length
      · rw [if_pos hr] at h
        cases hrow : reconRow tag bpp prev (rest.take rb) with
        | none => rw [hrow] at h; exact none_some h
        | some row =>
            rw [hrow] at h
            dsimp only at h
            cases hgo : pngDecodeGo bpp rb fuel row (rest.drop rb) with
            | none => rw [hgo] at h; exact none_some h
            | some out' =>
                rw [hgo] at h
                dsimp only at h
                injection h with e
                rw [← e]
                have ⟨k, hk1, hk2⟩ := pngDecodeGo_rows bpp rb fuel row (rest.drop rb) out' hgo
                have hrl : row.length = rb := by
                  rw [reconRow_length hrow]; exact length_take_of_le rb rest hr
                refine ⟨k + 1, ?_, ?_⟩
                · rw [length_append, hrl, hk1, Nat.succ_mul, Nat.add_comm]
                · rw [List.length_drop] at hk2
                  rw [Nat.succ_mul]
                  apply Nat.le_trans (Nat.add_le_add_right hk2 (rb + 1))
                  show rest.length - rb + (rb + 1) ≤ rest.length + 1
                  rw [← Nat.add_assoc, sub_add_cancel _ _ hr]
                  exact Nat.le_refl _
      · rw [if_neg hr] at h
        injection h with e
        rw [← e]
        exact ⟨0, (Nat.zero_mul rb).symm, by rw [Nat.zero_mul]; exact Nat.zero_le _⟩

/-- The decoded stream is whole rows of `rowBytes`, each from a complete encoded row of
    the data (tag byte included): nothing is read past the data, no partial row emitted. -/
theorem pngDecode_rows {colors bpc columns : Nat} {data out : List Nat}
    (h : pngDecode colors bpc columns data = some out) :
    ∃ k, out.length = k * rowBytes colors bpc columns ∧
      k * (rowBytes colors bpc columns + 1) ≤ data.length :=
  pngDecodeGo_rows _ _ _ _ _ _ h

/-- The bytes a FlateDecode/LZWDecode stream yields after its `/DecodeParms` predictor
    (Table 8): unchanged for `/Predictor 1`, PNG rows for 10 to 15 when the geometry is
    accepted. TIFF Predictor 2 is not decoded and, like an undefined `/Predictor`, refuses
    the stream: predicted bytes are never handed on as if they were data. -/
def unpredict (predictor colors bpc columns : Nat) (data : List Nat) : Option (List Nat) :=
  if predictorKind predictor = 0 then some data
  else if predictorKind predictor = 2 then
    if (pngGeometryFields colors bpc columns).1 = true then pngDecode colors bpc columns data
    else none
  else none

/-- Only `/Predictor 1` passes the data through; PNG output is whole rows of the data;
    anything else is refused. -/
theorem unpredict_cases (predictor colors bpc columns : Nat) (data out : List Nat)
    (h : unpredict predictor colors bpc columns data = some out) :
    (predictor = 1 ∧ out = data) ∨
      (10 ≤ predictor ∧ predictor ≤ 15 ∧ (pngGeometryFields colors bpc columns).1 = true ∧
        ∃ k, out.length = k * rowBytes colors bpc columns ∧
          k * (rowBytes colors bpc columns + 1) ≤ data.length) := by
  have ⟨k0, _, k2⟩ := predictorKind_cases predictor
  unfold unpredict at h
  by_cases h0 : predictorKind predictor = 0
  · rw [if_pos h0] at h
    injection h with e
    exact Or.inl ⟨k0.mp h0, e.symm⟩
  · rw [if_neg h0] at h
    by_cases h2 : predictorKind predictor = 2
    · rw [if_pos h2] at h
      by_cases hg : (pngGeometryFields colors bpc columns).1 = true
      · rw [if_pos hg] at h
        have ⟨hp1, hp2⟩ := k2.mp h2
        exact Or.inr ⟨hp1, hp2, hg, pngDecode_rows h⟩
      · rw [if_neg hg] at h; exact none_some h
    · rw [if_neg h2] at h; exact none_some h

/-! ## Examples -/

/-- Worked cases, checked by evaluation. The `/W [1 2 2]` rows are the shape of doc.pdf's
    cross-reference stream (object 24, entries for objects 0 to 24). -/
theorem examples :
    decodeEntry ⟨1, 2, 2⟩ [1, 0x22, 0x10, 0, 0] = .inUse 8720 0 ∧
    decodeEntry ⟨1, 2, 2⟩ [2, 0, 23, 0, 5] = .compressed 23 5 ∧
    decodeEntry ⟨1, 2, 2⟩ [0, 0, 0, 0xff, 0xff] = .free 0 65535 ∧
    decodeEntry ⟨1, 2, 2⟩ [7, 0, 1, 0, 0] = .null ∧
    decodeEntry ⟨0, 2, 0⟩ [1, 0] = .inUse 256 0 ∧
    rowLenFields 0 0 0 = (false, 0) ∧
    rowLenFields 1 2 2 = (true, 5) ∧
    (sectionEntries ⟨1, 2, 2⟩ [1, 0, 9, 0, 0, 2, 0, 3, 0, 1, 7] (defaultIndex 1000000)).length = 2 ∧
    beFold 0 [1, 0, 0, 0, 0, 0, 0, 0, 0] = none ∧
    beFold 0 [0, 0, 0, 0, 0, 0, 0, 0, 1] = some 1 ∧
    lookup [[(5, .free 0 1)], [(5, .inUse 10 0)]] 5 = .free 0 1 ∧
    lookup [[(4, .free 0 1)], [(5, .inUse 10 0)]] 5 = .inUse 10 0 ∧
    resolve 100 (fun n => if n = 3 then .compressed 7 1 else if n = 7 then .inUse 40 0 else .null)
      (fun n => if n = 7 then some 2 else none) 3 0 = .inStream 7 1 ∧
    resolve 100 (fun n => if n = 3 then .compressed 7 2 else if n = 7 then .inUse 40 0 else .null)
      (fun n => if n = 7 then some 2 else none) 3 0 = .absent ∧
    resolve 100 (fun n => if n = 3 then .compressed 7 0 else if n = 7 then .compressed 9 0 else .null)
      (fun n => if n = 7 then some 2 else none) 3 0 = .absent ∧
    resolve 100 (fun n => if n = 3 then .compressed 3 0 else .null)
      (fun n => if n = 3 then some 2 else none) 3 0 = .absent ∧
    resolve 100 (fun n => if n = 3 then .inUse 150 0 else .null) (fun _ => none) 3 0 = .absent ∧
    resolve 100 (fun n => if n = 3 then .inUse 40 1 else .null) (fun _ => none) 3 0 = .absent ∧
    resolve 100 (fun n => if n = 3 then .inUse 40 1 else .null) (fun _ => none) 3 1 = .direct 40 ∧
    headerOk ⟨20, 8, [(5, 0), (7, 4)]⟩ 2 = true ∧
    objSpan ⟨20, 8, [(5, 0), (7, 4)]⟩ 0 = some (8, 12) ∧
    objSpan ⟨20, 8, [(5, 0), (7, 4)]⟩ 1 = some (12, 20) ∧
    objSpan ⟨20, 8, [(5, 0), (7, 4)]⟩ 2 = none ∧
    headerOk ⟨20, 6, [(5, 0), (7, 4)]⟩ 2 = false ∧
    headerOk ⟨20, 8, [(5, 4), (7, 0)]⟩ 2 = false ∧
    headerOk ⟨20, 8, [(5, 0), (7, 12)]⟩ 2 = false ∧
    spanFields 8 30 false 0 20 = (false, 0, 0) ∧
    chainOk 100 [50, 10] = true ∧ chainOk 100 [10, 50, 10] = false ∧ chainOk 100 [100] = false ∧
    streamExtentFields 90 20 100 = (false, 0) ∧
    predictorKind 12 = 2 ∧ predictorKind 3 = 3 ∧ predictorKind 1 = 0 ∧
    rowBytes 1 8 5 = 5 ∧ rowBytes 3 8 10 = 30 ∧ rowBytes 1 1 9 = 2 ∧
    pixelBytes 3 16 = 6 ∧ pixelBytes 1 1 = 1 ∧
    pngGeometryFields 1 8 5 = (true, 5, 1) ∧ pngGeometryFields 1 3 5 = (false, 0, 0) ∧
    paeth 10 20 15 = 15 ∧ paeth 100 10 10 = 100 ∧
    reconRow 2 1 [1, 2, 3] [1, 1, 1] = some [2, 3, 4] ∧
    reconRow 1 1 [] [5, 1, 1] = some [5, 6, 7] ∧
    reconRow 1 1 [] [5, 255, 2] = some [5, 4, 6] ∧
    reconRow 3 1 [10, 20] [1, 2] = some [6, 15] ∧
    reconRow 7 1 [] [5] = none ∧
    pngDecode 1 8 3 [2, 1, 2, 3, 2, 1, 1, 1, 0, 9] = some [1, 2, 3, 2, 3, 4] :=
  ⟨by decide, by decide, by decide, by decide, by decide, by decide, by decide, by decide, by
    decide, by decide, by decide, by decide, by decide, by decide, by decide, by decide, by
    decide, by decide, by decide, by decide, by decide, by decide, by decide, by decide, by
    decide, by decide, by decide, by decide, by decide, by decide, by decide, by decide, by
    decide, by decide, by decide, by decide, by decide, by decide, by decide, by decide, by
    decide, by decide, by decide, by decide, by decide, by decide, by decide, by decide, by
    decide⟩

end PdfSpec
