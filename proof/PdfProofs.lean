import PdfSpec
import PdfMath

/-!
Refinement proofs for the Charon/Aeneas extraction of `proof/extract_pdf.rs` (`pdf_math`).

Each `*_refines` theorem states that the extracted `Result` function equals the field function
of `PdfSpec` on the words' `toNat` values, and each kernel-contract theorem states one
impossibility or bound that production relies on. Every proof is constructive: no `propext`,
`Quot.sound` or `Classical.choice` (checked by `proof/audit.py`, which has no allowlist).
-/

open WebfoldModel WebfoldModel.Std Result

namespace PdfProofs

open PdfSpec

/-! ## Constructive helpers -/

private theorem mod_of_lt {a b : Nat} (h : a < b) : a % b = a := by
  cases a with
  | zero => rfl
  | succ a => exact if_neg (Nat.not_le_of_gt h)

private theorem toNat_ofNat_lt {w n : Nat} (h : n < 2 ^ w) : (BitVec.ofNat w n).toNat = n :=
  mod_of_lt h

private theorem bind_ok {α β : Type} (x : α) (f : α → Result β) :
    (Bind.bind (Result.ok x) f : Result β) = f x := rfl

private theorem lift_ok {α : Type} (x : α) : (lift x : Result α) = Result.ok x := rfl

private theorem cast_u64 (x : Std.U8) :
    UScalar.cast UScalarTy.U64 x = BitVec.ofNat 64 x.toNat := rfl

private theorem cast_u8 (x : Std.U64) :
    UScalar.cast UScalarTy.U8 x = BitVec.ofNat 8 x.toNat := rfl

private theorem chk_add {w : Nat} (a b : BitVec w) (h : a.toNat + b.toNat < 2 ^ w) :
    (a + b : Result (BitVec w)) = Result.ok (BitVec.ofNat w (a.toNat + b.toNat)) := by
  show Std.checkedAdd a b = _
  unfold Std.checkedAdd
  exact if_pos h

private theorem chk_sub {w : Nat} (a b : BitVec w) (h : b.toNat ≤ a.toNat) :
    (a - b : Result (BitVec w)) = Result.ok (BitVec.ofNat w (a.toNat - b.toNat)) := by
  show Std.checkedSub a b = _
  unfold Std.checkedSub
  exact if_pos h

private theorem chk_mul {w : Nat} (a b : BitVec w) (h : a.toNat * b.toNat < 2 ^ w) :
    (a * b : Result (BitVec w)) = Result.ok (BitVec.ofNat w (a.toNat * b.toNat)) := by
  show Std.checkedMul a b = _
  unfold Std.checkedMul
  exact if_pos h

private theorem chk_div {w : Nat} (a b : BitVec w) (h : 0 < b.toNat) :
    (a / b : Result (BitVec w)) = Result.ok (BitVec.ofNat w (a.toNat / b.toNat)) := by
  show Std.checkedDiv a b = _
  unfold Std.checkedDiv
  exact if_neg (Nat.ne_of_gt h)

/-- A BitVec `≤` against a literal is the Nat comparison of the values. -/
private theorem bv_le_iff {w : Nat} (a : BitVec w) (n : Nat) (h : n < 2 ^ w) :
    (a ≤ BitVec.ofNat w n) ↔ a.toNat ≤ n := by
  show a.toNat ≤ (BitVec.ofNat w n).toNat ↔ _
  rw [toNat_ofNat_lt h]

/-- A BitVec `<` against a literal is the Nat comparison of the values. -/
private theorem bv_lt_iff {w : Nat} (a : BitVec w) (n : Nat) (h : n < 2 ^ w) :
    (a < BitVec.ofNat w n) ↔ a.toNat < n := by
  show a.toNat < (BitVec.ofNat w n).toNat ↔ _
  rw [toNat_ofNat_lt h]

private theorem ofNat_toNat_self {w : Nat} (x : BitVec w) : BitVec.ofNat w x.toNat = x :=
  BitVec.eq_of_toNat_eq (toNat_ofNat_lt x.isLt)

/-! ## Nat facts (only dependency-free lemmas: no `Nat.div_le_self`, `Nat.add_mul`, `omega`) -/

private theorem le_of_lt_pow {a : Nat} {w : Nat} (h : a < 2 ^ w) : a ≤ 2 ^ w :=
  Nat.le_of_lt h

/-- `i < d / l` exactly when `(i+1) * l <= d`, for a positive divisor. -/
private theorem lt_div_iff (i d l : Nat) (hl : 0 < l) :
    i < d / l ↔ (i + 1) * l ≤ d := by
  have hb := Arith.div_bounds d l hl
  constructor
  · intro hi
    have h1 : i + 1 ≤ d / l := hi
    have h2 : (i + 1) * l ≤ d / l * l := Nat.mul_le_mul_right l h1
    rw [Nat.mul_comm (d / l) l] at h2
    exact Nat.le_trans h2 hb.1
  · intro hd
    by_cases hc : i < d / l
    · exact hc
    · exfalso
      have h1 : d / l ≤ i := Nat.le_of_not_lt hc
      have h2 : l * (d / l) ≤ i * l := by
        rw [Nat.mul_comm i l]
        exact Nat.mul_le_mul_left l h1
      have h3 : (i + 1) * l = i * l + l := by
        rw [Arith.right_distrib, Nat.one_mul]
      have h4 : d < l * (d / l) + l := hb.2
      have h5 : l * (d / l) + l ≤ i * l + l := Nat.add_le_add_right h2 l
      have h6 : (i + 1) * l < (i + 1) * l :=
        Nat.lt_of_le_of_lt hd (Nat.lt_of_lt_of_le h4 (Nat.le_trans h5 (Nat.le_of_eq h3.symm)))
      exact Nat.lt_irrefl _ h6

private theorem bv_eq_of {w : Nat} (a : BitVec w) (n : Nat) (h : n < 2 ^ w) :
    a = BitVec.ofNat w n ↔ a.toNat = n :=
  ⟨fun e => by rw [e, toNat_ofNat_lt h], fun e => BitVec.eq_of_toNat_eq (by rw [toNat_ofNat_lt h]; exact e)⟩

/-- A BitVec `≤` with a literal on the left is the Nat comparison. -/
private theorem bv_le_iff' {w : Nat} (n : Nat) (a : BitVec w) (h : n < 2 ^ w) :
    (BitVec.ofNat w n ≤ a) ↔ n ≤ a.toNat := by
  show (BitVec.ofNat w n).toNat ≤ a.toNat ↔ _
  rw [toNat_ofNat_lt h]

/-- A BitVec `<` with a literal on the left is the Nat comparison. -/
private theorem bv_lt_iff' {w : Nat} (n : Nat) (a : BitVec w) (h : n < 2 ^ w) :
    (BitVec.ofNat w n < a) ↔ n < a.toNat := by
  show (BitVec.ofNat w n).toNat < a.toNat ↔ _
  rw [toNat_ofNat_lt h]

/-- `b <= c - a` is `a + b <= c` when `a <= c`. -/
private theorem le_sub_iff (a b c : Nat) (hac : a ≤ c) : b ≤ c - a ↔ a + b ≤ c := by
  constructor
  · intro h
    have e : c - a + a = c := Arith.sub_add_cancel c a hac
    have h2 : b + a ≤ c - a + a := Nat.add_le_add_right h a
    rw [e, Nat.add_comm] at h2
    exact h2
  · intro h
    exact Arith.le_sub_of_add_le h

/-- `n <= x / k` exactly when `n * k <= x`, for a positive divisor. -/
private theorem le_div_iff (n x k : Nat) (hk : 0 < k) : n ≤ x / k ↔ n * k ≤ x := by
  have hb := Arith.div_bounds x k hk
  constructor
  · intro h
    have h1 : n * k ≤ x / k * k := Nat.mul_le_mul_right k h
    rw [Nat.mul_comm (x / k) k] at h1
    exact Nat.le_trans h1 hb.1
  · intro h
    by_cases hc : n ≤ x / k
    · exact hc
    · exfalso
      have hlt : x / k < n := Nat.lt_of_not_le hc
      have h1 : (x / k + 1) * k ≤ n * k := Nat.mul_le_mul_right k hlt
      have h2 : (x / k + 1) * k = k * (x / k) + k := by
        rw [Arith.right_distrib, Nat.one_mul, Nat.mul_comm (x / k) k]
      have h4 : k * (x / k) + k ≤ x := by
        rw [← h2]
        exact Nat.le_trans h1 h
      exact Nat.lt_irrefl _ (Nat.lt_of_lt_of_le hb.2 h4)

/-- A quotient never exceeds its dividend. -/
private theorem div_le_self' (x k : Nat) (hk : 0 < k) : x / k ≤ x :=
  Nat.le_trans (Nat.le_mul_of_pos_right (x / k) hk)
    (by rw [Nat.mul_comm (x / k) k]; exact (Arith.div_bounds x k hk).1)

/-! ## Entry-type and field helpers -/

/-- `xref_entry_type`: the Table 18 code, with every unknown type collapsed to 3. -/
theorem xref_entry_type_refines (w1Zero : Bool) (f1 : Std.U64) :
    pdf_math.xref_entry_type w1Zero f1 =
      Result.ok (BitVec.ofNat 8 (entryTypeCode w1Zero f1.toNat)) := by
  unfold pdf_math.xref_entry_type entryTypeCode
  cases w1Zero
  · rw [if_neg Bool.false_ne_true, if_neg Bool.false_ne_true]
    by_cases h : f1.toNat ≤ 2
    · rw [if_pos ((bv_le_iff f1 2 (by decide)).2 h), if_pos h, cast_u8]
    · rw [if_neg (fun e => h ((bv_le_iff f1 2 (by decide)).1 e)), if_neg h]
  · rw [if_pos rfl, if_pos rfl]

/-- `xref_be_step`: one big-endian step, refused rather than wrapped at `2^64`. -/
theorem xref_be_step_refines (acc : Std.U64) (byte : Std.U8) :
    pdf_math.xref_be_step acc byte =
      Result.ok ((beStepFields acc.toNat byte.toNat).1,
        BitVec.ofNat 64 (beStepFields acc.toNat byte.toNat).2) := by
  unfold pdf_math.xref_be_step
  rw [cast_u64]
  have hb : byte.toNat < 256 := Nat.lt_of_lt_of_le byte.isLt (by decide)
  by_cases h : acc.toNat ≤ 72057594037927935
  · have hs : acc.toNat * 256 + byte.toNat < 2 ^ 64 :=
      Nat.lt_of_le_of_lt (Nat.add_le_add_right (Nat.mul_le_mul_right 256 h) byte.toNat)
        (Nat.lt_of_lt_of_le (Nat.add_lt_add_left hb (72057594037927935 * 256)) (by decide))
    have hspec : beStepFields acc.toNat byte.toNat = (true, acc.toNat * 256 + byte.toNat) := by
      unfold beStepFields
      exact if_pos hs
    have hm : acc.toNat * (256#u64).toNat < 2 ^ 64 := by
      show acc.toNat * 256 < 2 ^ 64
      exact Nat.lt_of_le_of_lt (Nat.mul_le_mul_right 256 h) (by decide)
    have hsum : (BitVec.ofNat 64 (acc.toNat * (256#u64).toNat)).toNat +
        (BitVec.ofNat 64 byte.toNat).toNat < 2 ^ 64 := by
      rw [toNat_ofNat_lt hm, toNat_ofNat_lt (Nat.lt_of_lt_of_le hb (by decide))]
      exact hs
    rw [if_pos ((bv_le_iff acc 72057594037927935 (by decide)).2 h)]
    rw [chk_mul acc 256#u64 hm, bind_ok, lift_ok, bind_ok, chk_add _ _ hsum, bind_ok]
    rw [toNat_ofNat_lt hm, toNat_ofNat_lt (Nat.lt_of_lt_of_le hb (by decide)), hspec]
    rfl
  · have hs : ¬ acc.toNat * 256 + byte.toNat < 2 ^ 64 := by
      intro hlt
      have hgt : 72057594037927935 < acc.toNat := Nat.lt_of_not_le h
      have h2 : 2 ^ 64 ≤ acc.toNat * 256 :=
        Nat.le_trans (by decide) (Nat.mul_le_mul_right 256 hgt)
      have h3 : 2 ^ 64 ≤ acc.toNat * 256 + byte.toNat := Nat.le_trans h2 (Nat.le_add_right _ _)
      exact Nat.lt_irrefl _ (Nat.lt_of_lt_of_le hlt h3)
    have hspec : beStepFields acc.toNat byte.toNat = (false, 0) := by
      unfold beStepFields
      exact if_neg hs
    rw [if_neg (fun e => h ((bv_le_iff acc 72057594037927935 (by decide)).1 e))]
    rw [hspec]

/-- `xref_row_len`: `/W` widths give a row length only when the sum is non-zero and fits in
    64 bits; every intermediate sum is checked. -/
theorem xref_row_len_refines (w1 w2 w3 : Std.U64) :
    pdf_math.xref_row_len w1 w2 w3 =
      Result.ok ((rowLenFields w1.toNat w2.toNat w3.toNat).1,
        BitVec.ofNat 64 (rowLenFields w1.toNat w2.toNat w3.toNat).2) := by
  unfold pdf_math.xref_row_len
  have hMt : (18446744073709551615#u64).toNat = 18446744073709551615 := rfl
  have hw1 : w1.toNat ≤ 18446744073709551615 := Nat.le_of_lt_succ w1.isLt
  have hi1 : 18446744073709551615 - w1.toNat < 2 ^ 64 :=
    Nat.lt_of_le_of_lt (Nat.sub_le _ _) (by decide)
  have hsub1 : w1.toNat ≤ (18446744073709551615#u64).toNat := by rw [hMt]; exact hw1
  rw [chk_sub _ _ hsub1, bind_ok, hMt]
  by_cases c1 : w1.toNat + w2.toNat ≤ 18446744073709551615
  · have hA : w1.toNat + w2.toNat < 2 ^ 64 := Nat.lt_of_le_of_lt c1 (by decide)
    rw [if_pos ((bv_le_iff w2 _ hi1).2 ((le_sub_iff _ _ _ hw1).2 c1)),
      chk_add w1 w2 hA, bind_ok]
    have hs12 : (BitVec.ofNat 64 (w1.toNat + w2.toNat)).toNat ≤
        (18446744073709551615#u64).toNat := by
      rw [hMt, toNat_ofNat_lt hA]
      exact c1
    rw [chk_sub (18446744073709551615#u64) (BitVec.ofNat 64 (w1.toNat + w2.toNat)) hs12,
      bind_ok, hMt, toNat_ofNat_lt hA]
    have hi2 : 18446744073709551615 - (w1.toNat + w2.toNat) < 2 ^ 64 :=
      Nat.lt_of_le_of_lt (Nat.sub_le _ _) (by decide)
    by_cases c2 : w1.toNat + w2.toNat + w3.toNat ≤ 18446744073709551615
    · have hB : w1.toNat + w2.toNat + w3.toNat < 2 ^ 64 := Nat.lt_of_le_of_lt c2 (by decide)
      have hC : (BitVec.ofNat 64 (w1.toNat + w2.toNat)).toNat + w3.toNat < 2 ^ 64 := by
        rw [toNat_ofNat_lt hA]
        exact hB
      have hsv : (BitVec.ofNat 64 ((BitVec.ofNat 64 (w1.toNat + w2.toNat)).toNat +
          w3.toNat)).toNat = w1.toNat + w2.toNat + w3.toNat := by
        rw [toNat_ofNat_lt hA, toNat_ofNat_lt hB]
      rw [if_pos ((bv_le_iff w3 _ hi2).2 ((le_sub_iff _ _ _ c1).2 c2)),
        chk_add _ w3 hC, bind_ok]
      by_cases hp : 0 < w1.toNat + w2.toNat + w3.toNat
      · rw [if_pos ((bv_lt_iff' 0 _ (by decide)).2 (by rw [hsv]; exact hp))]
        unfold rowLenFields
        rw [if_pos ⟨hp, hB⟩, toNat_ofNat_lt hA]
      · rw [if_neg (fun e => hp (by
          have := (bv_lt_iff' 0 _ (by decide)).1 e
          rw [hsv] at this
          exact this))]
        unfold rowLenFields
        rw [if_neg (fun h => hp h.1)]
    · rw [if_neg (fun e => c2 ((le_sub_iff _ _ _ c1).1 ((bv_le_iff w3 _ hi2).1 e)))]
      unfold rowLenFields
      rw [if_neg (fun h => c2 (Nat.le_of_lt_succ h.2))]
  · rw [if_neg (fun e => c1 ((le_sub_iff _ _ _ hw1).1 ((bv_le_iff w2 _ hi1).1 e)))]
    unfold rowLenFields
    rw [if_neg (fun h => c1 (Nat.le_trans (Nat.le_add_right _ _) (Nat.le_of_lt_succ h.2)))]

private theorem bv_le_def {w : Nat} (a b : BitVec w) : (a ≤ b) ↔ a.toNat ≤ b.toNat := Iff.rfl

private theorem bv_lt_def {w : Nat} (a b : BitVec w) : (a < b) ↔ a.toNat < b.toNat := Iff.rfl

/-- `row_start`: row `index` starts at `index * row_len` when it ends within the data. -/
theorem row_start_refines (index row_len data_len : Std.U64) :
    pdf_math.row_start index row_len data_len =
      Result.ok ((rowStartFields index.toNat row_len.toNat data_len.toNat).1,
        BitVec.ofNat 64 (rowStartFields index.toNat row_len.toNat data_len.toNat).2) := by
  unfold pdf_math.row_start
  by_cases hz : row_len.toNat = 0
  · rw [if_pos ((bv_eq_of row_len 0 (by decide)).2 hz)]
    have hs : rowStartFields index.toNat row_len.toNat data_len.toNat =
        (true, index.toNat * row_len.toNat) := by
      unfold rowStartFields
      rw [if_pos (by rw [hz, Nat.mul_zero]; exact Nat.zero_le _)]
    rw [hs, hz, Nat.mul_zero]
  · rw [if_neg (fun e => hz (congrArg BitVec.toNat e))]
    have hpos : 0 < row_len.toNat := Nat.pos_of_ne_zero hz
    have hq : data_len.toNat / row_len.toNat < 2 ^ 64 :=
      Nat.lt_of_le_of_lt (div_le_self' _ _ hpos) data_len.isLt
    rw [chk_div data_len row_len hpos, bind_ok]
    by_cases hi : index.toNat < data_len.toNat / row_len.toNat
    · have hd : (index.toNat + 1) * row_len.toNat ≤ data_len.toNat :=
        (lt_div_iff _ _ _ hpos).1 hi
      have hmul : index.toNat * row_len.toNat < 2 ^ 64 :=
        Nat.lt_of_le_of_lt (Nat.le_trans (Nat.mul_le_mul_right _ (Nat.le_succ _)) hd)
          data_len.isLt
      have hs : rowStartFields index.toNat row_len.toNat data_len.toNat =
          (true, index.toNat * row_len.toNat) := by
        unfold rowStartFields
        exact if_pos hd
      rw [if_pos ((bv_lt_iff index _ hq).2 hi), chk_mul index row_len hmul, bind_ok, hs]
    · have hs : rowStartFields index.toNat row_len.toNat data_len.toNat = (false, 0) := by
        unfold rowStartFields
        exact if_neg (fun hd => hi ((lt_div_iff _ _ _ hpos).2 hd))
      rw [if_neg (fun e => hi ((bv_lt_iff index _ hq).1 e)), hs]

/-- `objstm_ref_ok` is the Boolean conjunction of the four reference conditions. -/
theorem objstm_ref_ok_refines (sd isSelf hasN : Bool) (n index : Std.U64) :
    pdf_math.objstm_ref_ok sd isSelf hasN n index =
      Result.ok (objstmRefOk sd isSelf hasN n.toNat index.toNat) := by
  cases sd <;> cases isSelf <;> cases hasN <;> rfl

private theorem decide_iff {p q : Prop} [Decidable p] [Decidable q] (h : p ↔ q) :
    decide p = decide q := by
  by_cases hp : p
  · rw [decide_eq_true hp, decide_eq_true (h.1 hp)]
  · rw [decide_eq_false hp, decide_eq_false (fun hq => hp (h.2 hq))]

/-- `objstm_header_ok`: `/First` lies in the stream and the header can hold `N` pairs. -/
theorem objstm_header_ok_refines (first len n : Std.U64) :
    pdf_math.objstm_header_ok first len n =
      Result.ok (objstmHeaderFields first.toNat len.toNat n.toNat) := by
  unfold pdf_math.objstm_header_ok
  by_cases hfl : first.toNat ≤ len.toNat
  · have hnl : ¬ (len < first) := fun e =>
      Nat.lt_irrefl _ (Nat.lt_of_le_of_lt hfl ((bv_lt_def len first).1 e))
    rw [if_neg hnl]
    unfold objstmHeaderFields
    rw [decide_eq_true hfl, Bool.true_and]
    by_cases hM : first.toNat < 18446744073709551615
    · have hA1 : first.toNat + 1 < 2 ^ 64 := Nat.lt_of_le_of_lt hM (by decide)
      have hA : first.toNat + (1#u64).toNat < 2 ^ 64 := hA1
      have hq : (first.toNat + 1) / 4 < 2 ^ 64 :=
        Nat.lt_of_le_of_lt (div_le_self' _ _ (by decide)) hA1
      have hiff : n.toNat ≤ (first.toNat + 1) / 4 ↔ 4 * n.toNat ≤ first.toNat + 1 := by
        exact (le_div_iff _ _ 4 (by decide)).trans (by rw [Nat.mul_comm])
      rw [if_pos ((bv_lt_iff first 18446744073709551615 (by decide)).2 hM),
        chk_add first 1#u64 hA, bind_ok, chk_div _ 4#u64 (by decide), bind_ok,
        toNat_ofNat_lt hA]
      show Result.ok (decide (n ≤ BitVec.ofNat 64 ((first.toNat + 1) / 4))) =
        Result.ok (decide (4 * n.toNat ≤ first.toNat + 1))
      exact congrArg Result.ok (decide_iff ((bv_le_iff n _ hq).trans hiff))
    · have hfeq : first.toNat = 18446744073709551615 :=
        Nat.le_antisymm (Nat.le_of_lt_succ first.isLt) (Nat.not_lt.1 hM)
      rw [if_neg (fun e => hM ((bv_lt_iff first 18446744073709551615 (by decide)).1 e))]
      have hlow : n.toNat ≤ 4611686018427387904 ↔ 4 * n.toNat ≤ first.toNat + 1 := by
        rw [hfeq]
        constructor
        · intro h
          exact Nat.le_trans (Nat.mul_le_mul_left 4 h) (by decide)
        · intro h
          refine Nat.le_of_not_lt (fun hc => ?_)
          have h2 : 4 * (4611686018427387904 + 1) ≤ 4 * n.toNat :=
            Nat.mul_le_mul_left 4 hc
          exact absurd (Nat.le_trans h2 h) (by decide)
      exact congrArg Result.ok (decide_iff
        ((bv_le_iff n 4611686018427387904 (by decide)).trans hlow))
  · have hl : len < first := Nat.lt_of_not_le hfl
    unfold objstmHeaderFields
    rw [if_pos ((bv_lt_def len first).2 hl), decide_eq_false hfl]
    rfl

/-- `xref_offset_ok`: a byte offset must lie inside the file. -/
theorem xref_offset_ok_refines (offset file_len : Std.U64) :
    pdf_math.xref_offset_ok offset file_len = Result.ok (offsetOk offset.toNat file_len.toNat) :=
  congrArg Result.ok (decide_iff (bv_lt_def offset file_len))

/-- `xref_prev_ok`: a `/Prev` offset is followed only inside the file and only once. -/
theorem xref_prev_ok_refines (offset file_len : Std.U64) (seen : Bool) :
    pdf_math.xref_prev_ok offset file_len seen =
      Result.ok (prevOkFields offset.toNat file_len.toNat seen) := by
  unfold pdf_math.xref_prev_ok prevOkFields
  by_cases h : offset.toNat < file_len.toNat
  · rw [if_pos (show offset < file_len from h), decide_eq_true h]
    cases seen <;> rfl
  · rw [if_neg (show ¬ (offset < file_len) from h), decide_eq_false h]
    cases seen <;> rfl

/-- `stream_extent`: the `/Length` bytes from the data start must end inside the file. -/
theorem stream_extent_refines (start length file_len : Std.U64) :
    pdf_math.stream_extent start length file_len =
      Result.ok ((streamExtentFields start.toNat length.toNat file_len.toNat).1,
        BitVec.ofNat 64 (streamExtentFields start.toNat length.toNat file_len.toNat).2) := by
  unfold pdf_math.stream_extent
  by_cases hs : start.toNat ≤ file_len.toNat
  · rw [if_pos ((bv_le_def start file_len).2 hs), chk_sub file_len start hs, bind_ok]
    have hd : file_len.toNat - start.toNat < 2 ^ 64 :=
      Nat.lt_of_le_of_lt (Nat.sub_le _ _) file_len.isLt
    by_cases hsl : start.toNat + length.toNat ≤ file_len.toNat
    · have hA : start.toNat + length.toNat < 2 ^ 64 := Nat.lt_of_le_of_lt hsl file_len.isLt
      rw [if_pos ((bv_le_iff length _ hd).2 ((le_sub_iff _ _ _ hs).2 hsl)),
        chk_add start length hA, bind_ok]
      unfold streamExtentFields
      rw [if_pos hsl]
    · rw [if_neg (fun e => hsl ((le_sub_iff _ _ _ hs).1 ((bv_le_iff length _ hd).1 e)))]
      unfold streamExtentFields
      rw [if_neg hsl]
  · rw [if_neg (fun e => hs ((bv_le_def start file_len).1 e))]
    unfold streamExtentFields
    rw [if_neg (fun h => hs (Nat.le_trans (Nat.le_add_right _ _) h))]

/-- `predictor_kind`: `/Predictor` 1 is none, 2 is TIFF, 10 to 15 is PNG, else undefined. -/
theorem predictor_kind_refines (p : Std.U64) :
    pdf_math.predictor_kind p = Result.ok (BitVec.ofNat 8 (predictorKind p.toNat)) := by
  unfold pdf_math.predictor_kind predictorKind
  by_cases h1 : p.toNat = 1
  · rw [if_pos ((bv_eq_of p 1 (by decide)).2 h1), if_pos h1]
  · rw [if_neg (fun e => h1 ((bv_eq_of p 1 (by decide)).1 e)), if_neg h1]
    by_cases h2 : p.toNat = 2
    · rw [if_pos ((bv_eq_of p 2 (by decide)).2 h2), if_pos h2]
    · rw [if_neg (fun e => h2 ((bv_eq_of p 2 (by decide)).1 e)), if_neg h2]
      by_cases h10 : 10 ≤ p.toNat
      · rw [if_pos ((bv_le_iff' 10 p (by decide)).2 h10)]
        by_cases h15 : p.toNat ≤ 15
        · rw [if_pos ((bv_le_iff p 15 (by decide)).2 h15), if_pos ⟨h10, h15⟩]
        · rw [if_neg (fun e => h15 ((bv_le_iff p 15 (by decide)).1 e)),
            if_neg (fun h => h15 h.2)]
      · rw [if_neg (fun e => h10 ((bv_le_iff' 10 p (by decide)).1 e)),
          if_neg (fun h => h10 h.1)]

private theorem bv_lt_ofNat {w : Nat} (n m : Nat) (hn : n < 2 ^ w) (hm : m < 2 ^ w) :
    (BitVec.ofNat w n < BitVec.ofNat w m) ↔ n < m := by
  show (BitVec.ofNat w n).toNat < (BitVec.ofNat w m).toNat ↔ _
  rw [toNat_ofNat_lt hn, toNat_ofNat_lt hm]

private theorem spanFields_true' (f o n L : Nat) :
    spanFields f o true n L =
      if f + o < f + n ∧ f + n ≤ L then (true, f + o, f + n) else (false, 0, 0) := rfl

private theorem spanFields_false' (f o n L : Nat) :
    spanFields f o false n L =
      if f + o < L ∧ L ≤ L then (true, f + o, L) else (false, 0, 0) := rfl

/-- `objstm_span`: the extent of object `i` at `first + off` ends at the next object's start
    (or at `len`), and is accepted only when it is non-empty and inside the stream. -/
theorem objstm_span_refines (first off next len : Std.U64) (hn : Bool) :
    pdf_math.objstm_span first off hn next len =
      Result.ok ((spanFields first.toNat off.toNat hn next.toNat len.toNat).1,
        BitVec.ofNat 64 (spanFields first.toNat off.toNat hn next.toNat len.toNat).2.1,
        BitVec.ofNat 64 (spanFields first.toNat off.toNat hn next.toNat len.toNat).2.2) := by
  have hMt : (18446744073709551615#u64).toNat = 18446744073709551615 := rfl
  have hf : first.toNat ≤ 18446744073709551615 := Nat.le_of_lt_succ first.isLt
  have hfM : first.toNat ≤ (18446744073709551615#u64).toNat := by rw [hMt]; exact hf
  have hiM : 18446744073709551615 - first.toNat < 2 ^ 64 :=
    Nat.lt_of_le_of_lt (Nat.sub_le _ _) (by decide)
  have hL : len.toNat ≤ 18446744073709551615 := Nat.le_of_lt_succ len.isLt
  have hMlt : (18446744073709551615 : Nat) < 2 ^ 64 := by decide
  unfold pdf_math.objstm_span
  cases hn
  · rw [if_neg Bool.false_ne_true, chk_sub _ _ hfM, bind_ok, hMt, spanFields_false']
    by_cases hc2 : 18446744073709551615 - first.toNat < off.toNat
    · have hgt2 : 18446744073709551615 < first.toNat + off.toNat := by
        have h1 := Nat.add_lt_add_right hc2 first.toNat
        rw [Arith.sub_add_cancel _ _ hf, Nat.add_comm off.toNat] at h1
        exact h1
      rw [if_pos ((bv_lt_iff' _ off hiM).2 hc2)]
      rw [if_neg (fun h => absurd (Nat.lt_of_lt_of_le hgt2
        (Nat.le_of_lt (Nat.lt_of_lt_of_le h.1 hL))) (Nat.lt_irrefl _))]
    · have hfo : first.toNat + off.toNat ≤ 18446744073709551615 :=
        (le_sub_iff _ _ _ hf).1 (Nat.not_lt.1 hc2)
      have hB : first.toNat + off.toNat < 2 ^ 64 := Nat.lt_of_le_of_lt hfo hMlt
      rw [if_neg (fun e => hc2 ((bv_lt_iff' _ off hiM).1 e)), chk_add first off hB, bind_ok]
      by_cases hc3 : first.toNat + off.toNat < len.toNat
      · rw [if_pos ((bv_lt_iff' (first.toNat + off.toNat) len hB).2 hc3),
          if_pos ((bv_le_def len len).2 (Nat.le_refl _)),
          if_pos ⟨hc3, Nat.le_refl _⟩, ofNat_toNat_self len]
      · rw [if_neg (fun e => hc3 ((bv_lt_iff' (first.toNat + off.toNat) len hB).1 e)),
          if_neg (fun h => hc3 h.1)]
  · rw [if_pos rfl, chk_sub _ _ hfM, bind_ok, hMt, spanFields_true']
    by_cases hc1 : 18446744073709551615 - first.toNat < next.toNat
    · have hgt : 18446744073709551615 < first.toNat + next.toNat := by
        have h1 := Nat.add_lt_add_right hc1 first.toNat
        rw [Arith.sub_add_cancel _ _ hf, Nat.add_comm next.toNat] at h1
        exact h1
      rw [if_pos ((bv_lt_iff' _ next hiM).2 hc1)]
      rw [if_neg (fun h => absurd (Nat.lt_of_lt_of_le hgt h.2) (Nat.not_lt.2 hL))]
    · have hnx : first.toNat + next.toNat ≤ 18446744073709551615 :=
        (le_sub_iff _ _ _ hf).1 (Nat.not_lt.1 hc1)
      have hE : first.toNat + next.toNat < 2 ^ 64 := Nat.lt_of_le_of_lt hnx hMlt
      rw [if_neg (fun e => hc1 ((bv_lt_iff' _ next hiM).1 e)), chk_add first next hE, bind_ok]
      by_cases hc2 : 18446744073709551615 - first.toNat < off.toNat
      · have hgt2 : 18446744073709551615 < first.toNat + off.toNat := by
          have h1 := Nat.add_lt_add_right hc2 first.toNat
          rw [Arith.sub_add_cancel _ _ hf, Nat.add_comm off.toNat] at h1
          exact h1
        rw [if_pos ((bv_lt_iff' _ off hiM).2 hc2)]
        rw [if_neg (fun h => absurd (Nat.lt_of_lt_of_le hgt2
          (Nat.le_trans (Nat.le_of_lt h.1) (Nat.le_trans h.2 hL))) (Nat.lt_irrefl _))]
      · have hfo : first.toNat + off.toNat ≤ 18446744073709551615 :=
          (le_sub_iff _ _ _ hf).1 (Nat.not_lt.1 hc2)
        have hB : first.toNat + off.toNat < 2 ^ 64 := Nat.lt_of_le_of_lt hfo hMlt
        rw [if_neg (fun e => hc2 ((bv_lt_iff' _ off hiM).1 e)), chk_add first off hB, bind_ok]
        by_cases hc3 : first.toNat + off.toNat < first.toNat + next.toNat
        · by_cases hc4 : first.toNat + next.toNat ≤ len.toNat
          · rw [if_pos ((bv_lt_ofNat _ _ hB hE).2 hc3), if_pos ((bv_le_iff' _ len hE).2 hc4),
              if_pos ⟨hc3, hc4⟩]
          · rw [if_pos ((bv_lt_ofNat _ _ hB hE).2 hc3),
              if_neg (fun e => hc4 ((bv_le_iff' _ len hE).1 e)), if_neg (fun h => hc4 h.2)]
        · rw [if_neg (fun e => hc3 ((bv_lt_ofNat _ _ hB hE).1 e)), if_neg (fun h => hc3 h.1)]

private theorem bv_le_ofNat {w : Nat} (n m : Nat) (hn : n < 2 ^ w) (hm : m < 2 ^ w) :
    (BitVec.ofNat w n ≤ BitVec.ofNat w m) ↔ n ≤ m := by
  show (BitVec.ofNat w n).toNat ≤ (BitVec.ofNat w m).toNat ↔ _
  rw [toNat_ofNat_lt hn, toNat_ofNat_lt hm]

private theorem bpcOk_pos {b : Nat} (h : bpcOk b = true) : 0 < b := by
  apply Nat.pos_of_ne_zero
  intro e
  rw [e] at h
  exact absurd h (by decide)

/-- The `/BitsPerComponent` test (Table 8), as the join-point branches of the extraction. -/
private theorem bpc_join {α : Type} (bpc : Std.U64) (jp : Bool → Result α) :
    (if bpc = 1#u64 then Bind.bind (Result.ok true) jp
      else if bpc = 2#u64 then Bind.bind (Result.ok true) jp
      else if bpc = 4#u64 then Bind.bind (Result.ok true) jp
      else if bpc = 8#u64 then Bind.bind (Result.ok true) jp
      else if bpc = 16#u64 then Bind.bind (Result.ok true) jp
      else Bind.bind (Result.ok false) jp) = jp (bpcOk bpc.toNat) := by
  by_cases h1 : bpc.toNat = 1
  · rw [if_pos ((bv_eq_of bpc 1 (by decide)).2 h1), bind_ok]
    unfold bpcOk
    rw [decide_eq_true h1]
    rfl
  · rw [if_neg (fun e => h1 ((bv_eq_of bpc 1 (by decide)).1 e))]
    by_cases h2 : bpc.toNat = 2
    · rw [if_pos ((bv_eq_of bpc 2 (by decide)).2 h2), bind_ok]
      unfold bpcOk
      rw [decide_eq_false h1, decide_eq_true h2]
      rfl
    · rw [if_neg (fun e => h2 ((bv_eq_of bpc 2 (by decide)).1 e))]
      by_cases h4 : bpc.toNat = 4
      · rw [if_pos ((bv_eq_of bpc 4 (by decide)).2 h4), bind_ok]
        unfold bpcOk
        rw [decide_eq_false h1, decide_eq_false h2, decide_eq_true h4]
        rfl
      · rw [if_neg (fun e => h4 ((bv_eq_of bpc 4 (by decide)).1 e))]
        by_cases h8 : bpc.toNat = 8
        · rw [if_pos ((bv_eq_of bpc 8 (by decide)).2 h8), bind_ok]
          unfold bpcOk
          rw [decide_eq_false h1, decide_eq_false h2, decide_eq_false h4, decide_eq_true h8]
          rfl
        · rw [if_neg (fun e => h8 ((bv_eq_of bpc 8 (by decide)).1 e))]
          by_cases h16 : bpc.toNat = 16
          · rw [if_pos ((bv_eq_of bpc 16 (by decide)).2 h16), bind_ok]
            unfold bpcOk
            rw [decide_eq_false h1, decide_eq_false h2, decide_eq_false h4,
              decide_eq_false h8, decide_eq_true h16]
            rfl
          · rw [if_neg (fun e => h16 ((bv_eq_of bpc 16 (by decide)).1 e)), bind_ok]
            unfold bpcOk
            rw [decide_eq_false h1, decide_eq_false h2, decide_eq_false h4,
              decide_eq_false h8, decide_eq_false h16]
            rfl

/-- `png_geometry`: `(rowBytes, pixelBytes)` for Table 8 parameters whose row fits in 64 bits. -/
theorem png_geometry_refines (colors bpc columns : Std.U64) :
    pdf_math.png_geometry colors bpc columns =
      Result.ok ((pngGeometryFields colors.toNat bpc.toNat columns.toNat).1,
        BitVec.ofNat 64 (pngGeometryFields colors.toNat bpc.toNat columns.toNat).2.1,
        BitVec.ofNat 64 (pngGeometryFields colors.toNat bpc.toNat columns.toNat).2.2) := by
  have hMt : (18446744073709551615#u64).toNat = 18446744073709551615 := rfl
  have h7 : (7#u64).toNat = 7 := rfl
  unfold pdf_math.png_geometry
  dsimp only
  rw [bpc_join]
  unfold pngGeometryFields
  by_cases h1 : 1 ≤ colors.toNat
  · rw [if_pos ((bv_le_iff' 1 colors (by decide)).2 h1)]
    by_cases h2 : bpcOk bpc.toNat = true
    · rw [if_pos h2]
      by_cases h3 : 1 ≤ columns.toNat
      · rw [if_pos ((bv_le_iff' 1 columns (by decide)).2 h3)]
        have hb : 0 < bpc.toNat := bpcOk_pos h2
        have hc : 0 < columns.toNat := h3
        rw [chk_div (18446744073709551615#u64) bpc hb, bind_ok, hMt]
        have hq : 18446744073709551615 / bpc.toNat < 2 ^ 64 :=
          Nat.lt_of_le_of_lt (div_le_self' _ _ hb) (by decide)
        by_cases h4 : colors.toNat * bpc.toNat ≤ 18446744073709551615
        · have hcb : colors.toNat * bpc.toNat < 2 ^ 64 := Nat.lt_of_le_of_lt h4 (by decide)
          have hM7 : 18446744073709551615 - (7#u64).toNat < 2 ^ 64 := by decide
          have hX : (18446744073709551615 - (7#u64).toNat) / columns.toNat < 2 ^ 64 :=
            Nat.lt_of_le_of_lt (div_le_self' _ _ hc) hM7
          have key5 : colors.toNat * bpc.toNat * columns.toNat + 7 < 2 ^ 64 →
              colors.toNat * bpc.toNat ≤ (18446744073709551615 - (7#u64).toNat) / columns.toNat :=
            fun hl => (le_div_iff _ _ _ hc).2 ((le_sub_iff 7 _ _ (by decide)).2
              (by rw [Nat.add_comm]; exact Nat.le_of_lt_succ hl))
          rw [if_pos ((bv_le_iff colors _ hq).2 ((le_div_iff _ _ _ hb).2 h4)),
            chk_mul colors bpc hcb, bind_ok,
            chk_sub (18446744073709551615#u64) (7#u64) (by decide), bind_ok, hMt,
            chk_div _ columns hc, bind_ok, toNat_ofNat_lt hM7]
          by_cases h5 : colors.toNat * bpc.toNat ≤ (18446744073709551615 - (7#u64).toNat) / columns.toNat
          · rw [if_pos ((bv_le_ofNat _ _ hcb hX).2 h5)]
            have hq5 : colors.toNat * bpc.toNat * columns.toNat ≤ 18446744073709551615 - 7 :=
              (le_div_iff _ _ _ hc).1 h5
            have hlt : colors.toNat * bpc.toNat * columns.toNat + 7 < 2 ^ 64 := by
              have hsum : 7 + colors.toNat * bpc.toNat * columns.toNat ≤ 18446744073709551615 :=
                (le_sub_iff 7 _ _ (by decide)).1 hq5
              rw [Nat.add_comm]
              exact Nat.lt_of_le_of_lt hsum (by decide)
            have hrb : colors.toNat * bpc.toNat * columns.toNat < 2 ^ 64 :=
              Nat.lt_of_le_of_lt (Nat.le_add_right _ _) hlt
            have hA : (BitVec.ofNat 64 (colors.toNat * bpc.toNat)).toNat * columns.toNat < 2 ^ 64 := by
              rw [toNat_ofNat_lt hcb]
              exact hrb
            have hB : (BitVec.ofNat 64 (colors.toNat * bpc.toNat * columns.toNat)).toNat +
                (7#u64).toNat < 2 ^ 64 := by
              rw [toNat_ofNat_lt hrb, h7]
              exact hlt
            have hC : (BitVec.ofNat 64 (colors.toNat * bpc.toNat)).toNat + (7#u64).toNat < 2 ^ 64 := by
              rw [toNat_ofNat_lt hcb, h7]
              exact Nat.lt_of_le_of_lt (Nat.add_le_add_right
                (Nat.le_mul_of_pos_right _ hc) 7) hlt
            rw [chk_mul _ columns hA, bind_ok, toNat_ofNat_lt hcb, chk_add _ 7#u64 hB, bind_ok,
              chk_div _ 8#u64 (by decide), bind_ok, chk_add _ 7#u64 hC, bind_ok,
              chk_div _ 8#u64 (by decide), bind_ok]
            rw [if_pos ⟨h1, h2, h3, hlt⟩]
            have hcb7 : colors.toNat * bpc.toNat + 7 < 2 ^ 64 :=
              Nat.lt_of_le_of_lt (Nat.add_le_add_right (Nat.le_mul_of_pos_right _ hc) 7) hlt
            rw [toNat_ofNat_lt hrb, toNat_ofNat_lt hcb, h7, toNat_ofNat_lt hlt,
              toNat_ofNat_lt hcb7]
            rfl
          · rw [if_neg (fun e => h5 ((bv_le_ofNat _ _ hcb hX).1 e))]
            rw [if_neg (fun h => h5 (key5 h.2.2.2))]
        · rw [if_neg (fun e => h4 ((le_div_iff _ _ _ hb).1 ((bv_le_iff colors _ hq).1 e)))]
          rw [if_neg (fun h => h4 (Nat.le_of_lt_succ (Nat.lt_of_le_of_lt
            (Nat.le_trans (Nat.le_mul_of_pos_right _ hc) (Nat.le_add_right _ 7)) h.2.2.2)))]
      · rw [if_neg (fun e => h3 ((bv_le_iff' 1 columns (by decide)).1 e))]
        rw [if_neg (fun h => h3 h.2.2.1)]
    · rw [if_neg h2]
      rw [if_neg (fun h => h2 h.2.1)]
  · rw [if_neg (fun e => h1 ((bv_le_iff' 1 colors (by decide)).1 e))]
    rw [if_neg (fun h => h1 h.1)]


private theorem cast_u32 (x : Std.U8) :
    UScalar.cast UScalarTy.U32 x = BitVec.ofNat 32 x.toNat := rfl

private theorem cast_u8_32 (x : Std.U32) :
    UScalar.cast UScalarTy.U8 x = BitVec.ofNat 8 x.toNat := rfl

private theorem png_tail (x : Std.U8) (pr : Std.U32) (hp : pr.toNat < 256) :
    (do
      let i ← lift (UScalar.cast UScalarTy.U32 x)
      let sum ← i + pr
      if sum < 256#u32 then
        do
          let y ← ok sum
          let i1 ← lift (UScalar.cast UScalarTy.U8 y)
          ok (true, i1)
      else
        do
          let y ← sum - 256#u32
          let i1 ← lift (UScalar.cast UScalarTy.U8 y)
          ok (true, i1) : Result (Bool × Std.U8)) =
      Result.ok (true, BitVec.ofNat 8 (addByte x.toNat pr.toNat)) := by
  have hx : x.toNat < 256 := Nat.lt_of_lt_of_le x.isLt (by decide)
  have hA : x.toNat + pr.toNat < 512 := Nat.lt_of_lt_of_le (Nat.add_lt_add hx hp) (by decide)
  have hA32 : x.toNat + pr.toNat < 2 ^ 32 := Nat.lt_of_lt_of_le hA (by decide)
  have hx32 : x.toNat < 2 ^ 32 := Nat.lt_of_lt_of_le hx (by decide)
  have hsA : (BitVec.ofNat 32 x.toNat).toNat + pr.toNat < 2 ^ 32 := by
    rw [toNat_ofNat_lt hx32]
    exact hA32
  rw [lift_ok, cast_u32, bind_ok, chk_add _ pr hsA, bind_ok, toNat_ofNat_lt hx32]
  by_cases hs : x.toNat + pr.toNat < 256
  · rw [if_pos ((bv_lt_iff _ 256 (by decide)).2 (by rw [toNat_ofNat_lt hA32]; exact hs)),
      bind_ok, lift_ok, cast_u8_32, bind_ok, toNat_ofNat_lt hA32]
    unfold addByte
    rw [if_pos hs]
  · rw [if_neg (show ¬ (BitVec.ofNat 32 (x.toNat + pr.toNat) < 256#32) from fun e => by
        have h := (bv_lt_iff _ 256 (by decide)).1 e
        rw [toNat_ofNat_lt hA32] at h
        exact hs h)]
    have h256 : (256#u32).toNat ≤ (BitVec.ofNat 32 (x.toNat + pr.toNat)).toNat := by
      rw [toNat_ofNat_lt hA32]
      exact Nat.not_lt.1 hs
    rw [chk_sub _ _ h256, bind_ok, lift_ok, cast_u8_32, bind_ok, toNat_ofNat_lt hA32]
    unfold addByte
    rw [if_neg hs, show (256#u32).toNat = 256 from rfl,
      toNat_ofNat_lt (Nat.lt_of_le_of_lt (Nat.sub_le _ _) hA32)]


private theorem cast32_toNat (a : Std.U8) : (UScalar.cast UScalarTy.U32 a).toNat = a.toNat := by
  rw [cast_u32, toNat_ofNat_lt (Nat.lt_of_lt_of_le a.isLt (by decide))]

private theorem png_tail' (x : Std.U8) (pr : Std.U32) (n : Nat) (hn : pr.toNat = n) (hp : n < 256) :
    (do
      let i ← lift (UScalar.cast UScalarTy.U32 x)
      let sum ← i + pr
      if sum < 256#u32 then
        do
          let y ← ok sum
          let i1 ← lift (UScalar.cast UScalarTy.U8 y)
          ok (true, i1)
      else
        do
          let y ← sum - 256#u32
          let i1 ← lift (UScalar.cast UScalarTy.U8 y)
          ok (true, i1) : Result (Bool × Std.U8)) =
      Result.ok (true, BitVec.ofNat 8 (addByte x.toNat n)) := by
  rw [← hn]
  exact png_tail x pr (by rw [hn]; exact hp)

private theorem png_recon_k1 (x a b c : Std.U8) :
    pdf_math.png_recon_byte 1#8 x a b c = Result.ok (true, BitVec.ofNat 8 (addByte x.toNat a.toNat)) := by
  unfold pdf_math.png_recon_byte
  dsimp only
  rw [if_pos (show (1#8 ≤ 4#8) from by decide), lift_ok, bind_ok, lift_ok, bind_ok, lift_ok,
    bind_ok, if_neg (show ¬ ((1#8) = 0#8) from by decide), if_pos rfl, bind_ok]
  exact png_tail' x _ a.toNat (cast32_toNat a) (Nat.lt_of_lt_of_le a.isLt (by decide))

private theorem half_lt_256 (s : Nat) (h : s < 512) : s / 2 < 256 :=
  Nat.lt_of_not_le (fun hc => Nat.lt_irrefl 512 (Nat.lt_of_le_of_lt ((le_div_iff 256 s 2 (by decide)).1 hc) h))

private theorem png_recon_k3 (x a b c : Std.U8) :
    pdf_math.png_recon_byte 3#8 x a b c =
      Result.ok (true, BitVec.ofNat 8 (addByte x.toNat ((a.toNat + b.toNat) / 2))) := by
  unfold pdf_math.png_recon_byte
  dsimp only
  have hA : a.toNat < 256 := Nat.lt_of_lt_of_le a.isLt (by decide)
  have hB : b.toNat < 256 := Nat.lt_of_lt_of_le b.isLt (by decide)
  have hab : a.toNat + b.toNat < 512 := Nat.lt_of_lt_of_le (Nat.add_lt_add hA hB) (by decide)
  have hs : (UScalar.cast UScalarTy.U32 a).toNat + (UScalar.cast UScalarTy.U32 b).toNat < 2 ^ 32 := by
    rw [cast32_toNat, cast32_toNat]
    exact Nat.lt_of_lt_of_le (Nat.add_lt_add hA hB) (by decide)
  have hQ : ((BitVec.ofNat 32 ((UScalar.cast UScalarTy.U32 a).toNat +
      (UScalar.cast UScalarTy.U32 b).toNat)).toNat / (2#32).toNat) < 2 ^ 32 := by
    rw [toNat_ofNat_lt hs]
    exact Nat.lt_of_le_of_lt (div_le_self' _ _ (by decide)) hs
  rw [if_pos (show (3#8 ≤ 4#8) from by decide), lift_ok, bind_ok, lift_ok, bind_ok, lift_ok,
    bind_ok, if_neg (show ¬ ((3#8) = 0#8) from by decide),
    if_neg (show ¬ ((3#8) = 1#8) from by decide), if_neg (show ¬ ((3#8) = 2#8) from by decide),
    if_pos rfl, chk_add _ _ hs, bind_ok, chk_div _ 2#32 (by decide), bind_ok]
  exact png_tail' x _ ((a.toNat + b.toNat) / 2)
    (by
      rw [toNat_ofNat_lt hQ, toNat_ofNat_lt hs, cast32_toNat, cast32_toNat]
      rfl)
    (half_lt_256 _ hab)


private theorem absDiff_ofLe (x y : Nat) (h : y ≤ x) : absDiff x y = x - y := by
  unfold absDiff
  rw [Arith.sub_eq_zero_of_le y x h, Nat.add_zero]

private theorem absDiff_ofLt (x y : Nat) (h : x ≤ y) : absDiff x y = y - x := by
  unfold absDiff
  rw [Arith.sub_eq_zero_of_le x y h, Nat.zero_add]

/-- One absolute difference of 32-bit words, as the extraction computes it. -/
private theorem absK {α : Type} (a b : Std.U32) (jp : Std.U32 → Result α) :
    (if b ≥ a then Bind.bind (b - a) jp else Bind.bind (a - b) jp) =
      jp (BitVec.ofNat 32 (absDiff b.toNat a.toNat)) := by
  by_cases h : a ≤ b
  · have hN : a.toNat ≤ b.toNat := (bv_le_def a b).1 h
    rw [if_pos h, chk_sub b a hN, bind_ok, absDiff_ofLe b.toNat a.toNat hN]
  · have hN : ¬ a.toNat ≤ b.toNat := fun e => h ((bv_le_def a b).2 e)
    have hba : b.toNat ≤ a.toNat := Nat.le_of_lt (Nat.lt_of_not_le hN)
    rw [if_neg (show ¬ (b ≥ a) from h), chk_sub a b hba, bind_ok, absDiff_ofLt b.toNat a.toNat hba]


private theorem absDiff_le (x y : Nat) : absDiff x y ≤ x + y :=
  Nat.add_le_add (Nat.sub_le x y) (Nat.sub_le y x)

private theorem paeth_fst (a b c : Nat)
    (h : absDiff b c ≤ absDiff a c ∧ absDiff b c ≤ absDiff (a + b) (c + c)) : paeth a b c = a := by
  unfold paeth
  rw [if_pos h]

private theorem paeth_snd (a b c : Nat)
    (h1 : ¬ (absDiff b c ≤ absDiff a c ∧ absDiff b c ≤ absDiff (a + b) (c + c)))
    (h2 : absDiff a c ≤ absDiff (a + b) (c + c)) : paeth a b c = b := by
  unfold paeth
  rw [if_neg h1, if_pos h2]

private theorem paeth_thd (a b c : Nat)
    (h1 : ¬ (absDiff b c ≤ absDiff a c ∧ absDiff b c ≤ absDiff (a + b) (c + c)))
    (h2 : ¬ absDiff a c ≤ absDiff (a + b) (c + c)) : paeth a b c = c := by
  unfold paeth
  rw [if_neg h1, if_neg h2]

private theorem png_recon_k4 (x a b c : Std.U8) :
    pdf_math.png_recon_byte 4#8 x a b c =
      Result.ok (true, BitVec.ofNat 8 (addByte x.toNat (paeth a.toNat b.toNat c.toNat))) := by
  unfold pdf_math.png_recon_byte
  dsimp only
  have hA : a.toNat < 256 := Nat.lt_of_lt_of_le a.isLt (by decide)
  have hB : b.toNat < 256 := Nat.lt_of_lt_of_le b.isLt (by decide)
  have hC : c.toNat < 256 := Nat.lt_of_lt_of_le c.isLt (by decide)
  have hsab : (UScalar.cast UScalarTy.U32 a).toNat + (UScalar.cast UScalarTy.U32 b).toNat < 2 ^ 32 := by
    rw [cast32_toNat, cast32_toNat]
    exact Nat.lt_of_lt_of_le (Nat.add_lt_add hA hB) (by decide)
  have hscc : (UScalar.cast UScalarTy.U32 c).toNat + (UScalar.cast UScalarTy.U32 c).toNat < 2 ^ 32 := by
    rw [cast32_toNat]
    exact Nat.lt_of_lt_of_le (Nat.add_lt_add hC hC) (by decide)
  rw [if_pos (show (4#8 ≤ 4#8) from by decide), lift_ok, bind_ok, lift_ok, bind_ok, lift_ok,
    bind_ok, if_neg (show ¬ ((4#8) = 0#8) from by decide),
    if_neg (show ¬ ((4#8) = 1#8) from by decide), if_neg (show ¬ ((4#8) = 2#8) from by decide),
    if_neg (show ¬ ((4#8) = 3#8) from by decide)]
  rw [absK (UScalar.cast UScalarTy.U32 c) (UScalar.cast UScalarTy.U32 b)]
  rw [absK (UScalar.cast UScalarTy.U32 c) (UScalar.cast UScalarTy.U32 a)]
  rw [chk_add _ _ hsab, bind_ok, chk_add _ _ hscc, bind_ok]
  rw [absK _ _]
  rw [toNat_ofNat_lt hsab, toNat_ofNat_lt hscc, cast32_toNat a, cast32_toNat b, cast32_toNat c]
  have hd_bc : absDiff b.toNat c.toNat < 2 ^ 32 :=
    Nat.lt_of_le_of_lt (absDiff_le _ _) (Nat.lt_of_lt_of_le (Nat.add_lt_add hB hC) (by decide))
  have hd_ac : absDiff a.toNat c.toNat < 2 ^ 32 :=
    Nat.lt_of_le_of_lt (absDiff_le _ _) (Nat.lt_of_lt_of_le (Nat.add_lt_add hA hC) (by decide))
  have hd_pc : absDiff (a.toNat + b.toNat) (c.toNat + c.toNat) < 2 ^ 32 :=
    Nat.lt_of_le_of_lt (absDiff_le _ _)
      (Nat.lt_of_lt_of_le (Nat.add_lt_add (Nat.add_lt_add hA hB) (Nat.add_lt_add hC hC)) (by decide))
  by_cases p1 : absDiff b.toNat c.toNat ≤ absDiff a.toNat c.toNat
  · rw [if_pos ((bv_le_ofNat _ _ hd_bc hd_ac).2 p1)]
    by_cases p2 : absDiff b.toNat c.toNat ≤ absDiff (a.toNat + b.toNat) (c.toNat + c.toNat)
    · rw [if_pos ((bv_le_ofNat _ _ hd_bc hd_pc).2 p2), bind_ok, paeth_fst _ _ _ ⟨p1, p2⟩]
      exact png_tail' x _ a.toNat (cast32_toNat a) hA
    · rw [if_neg (fun e => p2 ((bv_le_ofNat _ _ hd_bc hd_pc).1 e))]
      by_cases p3 : absDiff a.toNat c.toNat ≤ absDiff (a.toNat + b.toNat) (c.toNat + c.toNat)
      · rw [if_pos ((bv_le_ofNat _ _ hd_ac hd_pc).2 p3), bind_ok,
          paeth_snd _ _ _ (fun h => p2 h.2) p3]
        exact png_tail' x _ b.toNat (cast32_toNat b) hB
      · rw [if_neg (fun e => p3 ((bv_le_ofNat _ _ hd_ac hd_pc).1 e)), bind_ok,
          paeth_thd _ _ _ (fun h => p2 h.2) p3]
        exact png_tail' x _ c.toNat (cast32_toNat c) hC
  · rw [if_neg (fun e => p1 ((bv_le_ofNat _ _ hd_bc hd_ac).1 e))]
    by_cases p3 : absDiff a.toNat c.toNat ≤ absDiff (a.toNat + b.toNat) (c.toNat + c.toNat)
    · rw [if_pos ((bv_le_ofNat _ _ hd_ac hd_pc).2 p3), bind_ok,
        paeth_snd _ _ _ (fun h => p1 h.1) p3]
      exact png_tail' x _ b.toNat (cast32_toNat b) hB
    · rw [if_neg (fun e => p3 ((bv_le_ofNat _ _ hd_ac hd_pc).1 e)), bind_ok,
        paeth_thd _ _ _ (fun h => p1 h.1) p3]
      exact png_tail' x _ c.toNat (cast32_toNat c) hC


private theorem png_recon_k0 (x _a _b _c : Std.U8) :
    pdf_math.png_recon_byte 0#8 x _a _b _c = Result.ok (true, BitVec.ofNat 8 (addByte x.toNat 0)) := by
  unfold pdf_math.png_recon_byte
  dsimp only
  rw [if_pos (show (0#8 ≤ 4#8) from by decide), lift_ok, bind_ok, lift_ok, bind_ok, lift_ok,
    bind_ok, if_pos rfl, bind_ok]
  exact png_tail' x 0#32 0 rfl (by decide)

private theorem png_recon_k2 (x a b _c : Std.U8) :
    pdf_math.png_recon_byte 2#8 x a b _c = Result.ok (true, BitVec.ofNat 8 (addByte x.toNat b.toNat)) := by
  unfold pdf_math.png_recon_byte
  dsimp only
  rw [if_pos (show (2#8 ≤ 4#8) from by decide), lift_ok, bind_ok, lift_ok, bind_ok, lift_ok,
    bind_ok, if_neg (show ¬ ((2#8) = 0#8) from by decide),
    if_neg (show ¬ ((2#8) = 1#8) from by decide), if_pos rfl, bind_ok]
  exact png_tail' x _ b.toNat (cast32_toNat b) (Nat.lt_of_lt_of_le b.isLt (by decide))

private theorem le4_cases (t : Nat) (h : t ≤ 4) : t = 0 ∨ t = 1 ∨ t = 2 ∨ t = 3 ∨ t = 4 := by
  cases t with
  | zero => exact Or.inl rfl
  | succ t1 =>
    cases t1 with
    | zero => exact Or.inr (Or.inl rfl)
    | succ t2 =>
      cases t2 with
      | zero => exact Or.inr (Or.inr (Or.inl rfl))
      | succ t3 =>
        cases t3 with
        | zero => exact Or.inr (Or.inr (Or.inr (Or.inl rfl)))
        | succ t4 =>
          cases t4 with
          | zero => exact Or.inr (Or.inr (Or.inr (Or.inr rfl)))
          | succ t5 => exact absurd (Nat.le_trans (Nat.le_add_left 5 t5) h) (by decide)

private theorem reconFields_some (t x a b c p : Nat) (h : predict t a b c = some p) :
    reconFields t x a b c = (true, addByte x p) := by
  unfold reconFields reconByte
  rw [h]
  rfl

/-- `png_recon_byte`: the PNG reconstruction of one byte, for every filter tag. -/
theorem png_recon_byte_refines (tag x a b c : Std.U8) :
    pdf_math.png_recon_byte tag x a b c =
      Result.ok ((reconFields tag.toNat x.toNat a.toNat b.toNat c.toNat).1,
        BitVec.ofNat 8 (reconFields tag.toNat x.toNat a.toNat b.toNat c.toNat).2) := by
  by_cases ht : tag.toNat ≤ 4
  · rcases le4_cases tag.toNat ht with h | h | h | h | h
    · have e : tag = 0#8 := (bv_eq_of tag 0 (by decide)).2 h
      subst e
      rw [png_recon_k0, reconFields_some _ _ _ _ _ 0 rfl]
    · have e : tag = 1#8 := (bv_eq_of tag 1 (by decide)).2 h
      subst e
      rw [png_recon_k1, reconFields_some _ _ _ _ _ a.toNat rfl]
    · have e : tag = 2#8 := (bv_eq_of tag 2 (by decide)).2 h
      subst e
      rw [png_recon_k2, reconFields_some _ _ _ _ _ b.toNat rfl]
    · have e : tag = 3#8 := (bv_eq_of tag 3 (by decide)).2 h
      subst e
      rw [png_recon_k3, reconFields_some _ _ _ _ _ ((a.toNat + b.toNat) / 2) rfl]
    · have e : tag = 4#8 := (bv_eq_of tag 4 (by decide)).2 h
      subst e
      rw [png_recon_k4, reconFields_some _ _ _ _ _ (paeth a.toNat b.toNat c.toNat) rfl]
  · unfold pdf_math.png_recon_byte
    rw [if_neg (fun e => ht ((bv_le_iff tag 4 (by decide)).1 e)),
      (recon_invalid_tag x.toNat a.toNat b.toNat c.toNat (Nat.lt_of_not_le ht)).2]

/-! ## Kernel contracts: what production relies on -/

private theorem beStep_true {a b : Nat} (h : (beStepFields a b).1 = true) :
    (beStepFields a b).2 = a * 256 + b ∧ a * 256 + b < 2 ^ 64 := by
  unfold beStepFields at h ⊢
  by_cases hs : a * 256 + b < 2 ^ 64
  · rw [if_pos hs]
    exact ⟨rfl, hs⟩
  · rw [if_neg hs] at h
    exact absurd h (by decide)

/-- A big-endian step that is accepted never wraps: the value is exactly `acc * 256 + byte`. -/
theorem xref_kernel_be_step_no_wrap (acc : Std.U64) (byte : Std.U8) (v : Std.U64)
    (h : pdf_math.xref_be_step acc byte = Result.ok (true, v)) :
    v.toNat = acc.toNat * 256 + byte.toNat ∧ acc.toNat * 256 + byte.toNat < 2 ^ 64 := by
  rw [xref_be_step_refines] at h
  have h2 := Result.ok.inj h
  have hb : (beStepFields acc.toNat byte.toNat).1 = true := congrArg Prod.fst h2
  have hv : BitVec.ofNat 64 (beStepFields acc.toNat byte.toNat).2 = v := congrArg Prod.snd h2
  obtain ⟨he, hlt⟩ := beStep_true hb
  rw [← hv, he, toNat_ofNat_lt hlt]
  exact ⟨rfl, hlt⟩

/-- An all-zero `/W` is refused, so `/Size` never drives the entry loop. -/
theorem xref_kernel_row_zero_refused (w1 w2 w3 : Std.U64)
    (h : w1.toNat + w2.toNat + w3.toNat = 0) :
    pdf_math.xref_row_len w1 w2 w3 = Result.ok (false, 0#u64) := by
  rw [xref_row_len_refines, rowLenFields_zero_refused _ _ _ h]

/-- An accepted row starts at `index * row_len` and ends inside the data. -/
theorem xref_kernel_row_within_data (index row_len data_len s : Std.U64)
    (h : pdf_math.row_start index row_len data_len = Result.ok (true, s)) :
    s.toNat = index.toNat * row_len.toNat ∧ s.toNat + row_len.toNat ≤ data_len.toNat := by
  rw [row_start_refines] at h
  have h2 := Result.ok.inj h
  have hb : (rowStartFields index.toNat row_len.toNat data_len.toNat).1 = true :=
    congrArg Prod.fst h2
  have hv : BitVec.ofNat 64 (rowStartFields index.toNat row_len.toNat data_len.toNat).2 = s :=
    congrArg Prod.snd h2
  have hpair : rowStartFields index.toNat row_len.toNat data_len.toNat =
      (true, (rowStartFields index.toNat row_len.toNat data_len.toNat).2) := Prod.ext hb rfl
  obtain ⟨e1, e2⟩ := rowStart_within hpair
  have hlt : (rowStartFields index.toNat row_len.toNat data_len.toNat).2 < 2 ^ 64 :=
    Nat.lt_of_le_of_lt (Nat.le_trans (Nat.le_add_right _ _) e2) data_len.isLt
  rw [← hv, toNat_ofNat_lt hlt]
  exact ⟨e1, e2⟩

/-- A byte offset at or past the end of the file is refused. -/
theorem xref_kernel_offset_outside_refused (offset file_len : Std.U64)
    (h : file_len.toNat ≤ offset.toNat) :
    pdf_math.xref_offset_ok offset file_len = Result.ok false := by
  rw [xref_offset_ok_refines]
  unfold offsetOk
  rw [decide_eq_false (Nat.not_lt.2 h)]

/-- A type field above 2 names the null object. -/
theorem xref_kernel_unknown_type_null (f1 : Std.U64) (h : 2 < f1.toNat) :
    pdf_math.xref_entry_type false f1 = Result.ok 3#u8 := by
  rw [xref_entry_type_refines]
  unfold entryTypeCode
  rw [if_neg Bool.false_ne_true, if_neg (Nat.not_le.2 h)]

/-- A compressed reference at or past `/N` is refused. -/
theorem objstm_kernel_index_at_n_refused (sd isSelf hasN : Bool) (n index : Std.U64)
    (h : n.toNat ≤ index.toNat) :
    pdf_math.objstm_ref_ok sd isSelf hasN n index = Result.ok false := by
  rw [objstm_ref_ok_refines]
  unfold objstmRefOk
  rw [decide_eq_false (Nat.not_lt.2 h), Bool.and_false]

/-- A stream that is not an in-file object in use is never read from. -/
theorem objstm_kernel_stream_not_direct_refused (isSelf hasN : Bool) (n index : Std.U64) :
    pdf_math.objstm_ref_ok false isSelf hasN n index = Result.ok false := by
  rw [objstm_ref_ok_refines]
  cases isSelf <;> cases hasN <;> rfl

/-- An object stream never resolves a reference to itself. -/
theorem objstm_kernel_self_refused (sd hasN : Bool) (n index : Std.U64) :
    pdf_math.objstm_ref_ok sd true hasN n index = Result.ok false := by
  rw [objstm_ref_ok_refines]
  cases sd <;> cases hasN <;> rfl

/-- An accepted object extent lies inside the stream and is non-empty. -/
theorem objstm_kernel_span_within (first off next len s e : Std.U64) (hn : Bool)
    (h : pdf_math.objstm_span first off hn next len = Result.ok (true, s, e)) :
    first.toNat ≤ s.toNat ∧ s.toNat < e.toNat ∧ e.toNat ≤ len.toNat := by
  rw [objstm_span_refines] at h
  have h2 := Result.ok.inj h
  have hb : (spanFields first.toNat off.toNat hn next.toNat len.toNat).1 = true :=
    congrArg Prod.fst h2
  have hs : BitVec.ofNat 64 (spanFields first.toNat off.toNat hn next.toNat len.toNat).2.1 = s :=
    congrArg (fun p => p.2.1) h2
  have he : BitVec.ofNat 64 (spanFields first.toNat off.toNat hn next.toNat len.toNat).2.2 = e :=
    congrArg (fun p => p.2.2) h2
  have hpair : spanFields first.toNat off.toNat hn next.toNat len.toNat =
      (true, (spanFields first.toNat off.toNat hn next.toNat len.toNat).2.1,
        (spanFields first.toNat off.toNat hn next.toNat len.toNat).2.2) :=
    Prod.ext hb (Prod.ext rfl rfl)
  obtain ⟨_, f1, f2, f3⟩ := spanFields_within hpair
  have hel : (spanFields first.toNat off.toNat hn next.toNat len.toNat).2.2 < 2 ^ 64 :=
    Nat.lt_of_le_of_lt f3 len.isLt
  have hsl : (spanFields first.toNat off.toNat hn next.toNat len.toNat).2.1 < 2 ^ 64 :=
    Nat.lt_of_lt_of_le (Nat.lt_of_lt_of_le f2 f3) (Nat.le_of_lt len.isLt)
  have hsv : s.toNat = (spanFields first.toNat off.toNat hn next.toNat len.toNat).2.1 := by
    rw [← hs, toNat_ofNat_lt hsl]
  have hev : e.toNat = (spanFields first.toNat off.toNat hn next.toNat len.toNat).2.2 := by
    rw [← he, toNat_ofNat_lt hel]
  rw [hsv, hev]
  exact ⟨f1, f2, f3⟩

/-- An accepted object-stream header has `N` no larger than `/First`, which is in the stream. -/
theorem objstm_kernel_header_bounds_n (first len n : Std.U64)
    (h : pdf_math.objstm_header_ok first len n = Result.ok true) :
    n.toNat ≤ first.toNat ∧ first.toNat ≤ len.toNat := by
  rw [objstm_header_ok_refines] at h
  exact objstmHeader_bounds_n (Result.ok.inj h)

/-- A section offset that was already read is never followed again. -/
theorem xref_kernel_prev_revisit_refused (offset file_len : Std.U64) :
    pdf_math.xref_prev_ok offset file_len true = Result.ok false := by
  rw [xref_prev_ok_refines]
  unfold prevOkFields
  rw [Bool.not_true, Bool.and_false]

/-- A section offset at or past the end of the file is never followed. -/
theorem xref_kernel_prev_outside_refused (offset file_len : Std.U64) (seen : Bool)
    (h : file_len.toNat ≤ offset.toNat) :
    pdf_math.xref_prev_ok offset file_len seen = Result.ok false := by
  rw [xref_prev_ok_refines]
  unfold prevOkFields
  rw [decide_eq_false (Nat.not_lt.2 h), Bool.false_and]

/-- An accepted stream extent ends at `start + /Length` inside the file. -/
theorem stream_kernel_extent_within (start length file e : Std.U64)
    (h : pdf_math.stream_extent start length file = Result.ok (true, e)) :
    e.toNat = start.toNat + length.toNat ∧ start.toNat ≤ e.toNat ∧ e.toNat ≤ file.toNat := by
  rw [stream_extent_refines] at h
  have h2 := Result.ok.inj h
  have hb : (streamExtentFields start.toNat length.toNat file.toNat).1 = true :=
    congrArg Prod.fst h2
  have hv : BitVec.ofNat 64 (streamExtentFields start.toNat length.toNat file.toNat).2 = e :=
    congrArg Prod.snd h2
  have hpair : streamExtentFields start.toNat length.toNat file.toNat =
      (true, (streamExtentFields start.toNat length.toNat file.toNat).2) := Prod.ext hb rfl
  obtain ⟨f1, f2, f3⟩ := streamExtent_within hpair
  have hlt : (streamExtentFields start.toNat length.toNat file.toNat).2 < 2 ^ 64 :=
    Nat.lt_of_le_of_lt f3 file.isLt
  rw [← hv, toNat_ofNat_lt hlt]
  exact ⟨f1, f2, f3⟩

/-- A PNG filter tag above 4 refuses the byte. -/
theorem png_kernel_invalid_tag_refused (tag x a b c : Std.U8) (h : 5 ≤ tag.toNat) :
    pdf_math.png_recon_byte tag x a b c = Result.ok (false, 0#u8) := by
  rw [png_recon_byte_refines, (recon_invalid_tag x.toNat a.toNat b.toNat c.toNat h).2]

/-- Filtering then reconstructing a byte returns the original byte. -/
theorem png_kernel_recon_roundtrip (tag x a b c filt : Std.U8)
    (h : filterByte tag.toNat x.toNat a.toNat b.toNat c.toNat = some filt.toNat) :
    pdf_math.png_recon_byte tag filt a b c = Result.ok (true, x) := by
  rw [png_recon_byte_refines]
  have hx : x.toNat < 256 := Nat.lt_of_lt_of_le x.isLt (by decide)
  have hA : a.toNat < 256 := Nat.lt_of_lt_of_le a.isLt (by decide)
  have hB : b.toNat < 256 := Nat.lt_of_lt_of_le b.isLt (by decide)
  have hC : c.toNat < 256 := Nat.lt_of_lt_of_le c.isLt (by decide)
  have hr : reconByte tag.toNat filt.toNat a.toNat b.toNat c.toNat = some x.toNat :=
    recon_filter_roundtrip hx hA hB hC h
  have hf : reconFields tag.toNat filt.toNat a.toNat b.toNat c.toNat = (true, x.toNat) :=
    (reconFields_spec _ _ _ _ _ _).2 hr
  rw [hf, ofNat_toNat_self]

/-- An accepted PNG geometry has row bytes covering exactly the row's bits. -/
theorem png_kernel_geometry_bits (colors bpc columns rb pb : Std.U64)
    (h : pdf_math.png_geometry colors bpc columns = Result.ok (true, rb, pb)) :
    colors.toNat * bpc.toNat * columns.toNat ≤ 8 * rb.toNat ∧
      8 * rb.toNat < colors.toNat * bpc.toNat * columns.toNat + 8 := by
  rw [png_geometry_refines] at h
  have h2 := Result.ok.inj h
  have hb : (pngGeometryFields colors.toNat bpc.toNat columns.toNat).1 = true :=
    congrArg Prod.fst h2
  have hrb : BitVec.ofNat 64 (pngGeometryFields colors.toNat bpc.toNat columns.toNat).2.1 = rb :=
    congrArg (fun p => p.2.1) h2
  have hc : 1 ≤ colors.toNat ∧ bpcOk bpc.toNat = true ∧ 1 ≤ columns.toNat ∧
      colors.toNat * bpc.toNat * columns.toNat + 7 < 2 ^ 64 := by
    by_cases hn : 1 ≤ colors.toNat ∧ bpcOk bpc.toNat = true ∧ 1 ≤ columns.toNat ∧
        colors.toNat * bpc.toNat * columns.toNat + 7 < 2 ^ 64
    · exact hn
    · unfold pngGeometryFields at hb
      rw [if_neg hn] at hb
      exact absurd hb (by decide)
  have hpg : pngGeometryFields colors.toNat bpc.toNat columns.toNat =
      (true, rowBytes colors.toNat bpc.toNat columns.toNat,
        pixelBytes colors.toNat bpc.toNat) := by
    unfold pngGeometryFields
    exact if_pos hc
  rw [hpg] at hrb
  have hbound : colors.toNat * bpc.toNat * columns.toNat + 7 < 2 ^ 64 := hc.2.2.2
  have hrlt : rowBytes colors.toNat bpc.toNat columns.toNat < 2 ^ 64 := by
    unfold rowBytes
    exact Nat.lt_of_le_of_lt (div_le_self' _ _ (by decide)) hbound
  have hrbv : rb.toNat = rowBytes colors.toNat bpc.toNat columns.toNat := by
    rw [← hrb]
    exact toNat_ofNat_lt hrlt
  rw [hrbv]
  exact rowBytes_bits colors.toNat bpc.toNat columns.toNat

/-- An undefined `/Predictor` value refuses the stream. -/
theorem predictor_kernel_undefined_refused (p : Std.U64) (h1 : p.toNat ≠ 1) (h2 : p.toNat ≠ 2)
    (h3 : ¬ (10 ≤ p.toNat ∧ p.toNat ≤ 15)) :
    pdf_math.predictor_kind p = Result.ok 3#u8 := by
  rw [predictor_kind_refines]
  unfold predictorKind
  rw [if_neg h1, if_neg h2, if_neg h3]

end PdfProofs
