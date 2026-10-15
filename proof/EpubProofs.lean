import EpubSpec
import EpubMath
import EpubTables

/-!
Refinement proofs for the Charon/Aeneas extraction of `proof/extract_epub.rs` (`epub_math`).

Each `*_refines` theorem states that the extracted `Result` function equals the field function
of `EpubSpec` on the words' `toNat` values. The byte-valued kernels (`opf_child_role`,
`href_byte_class`, `pct_hex`) are checked over every input in `EpubTables`; the others are
proved structurally here. The contract theorems at the end state what production relies on,
as consequences of those refinements and of the specification. Every proof is constructive:
no `propext`, `Quot.sound` or `Classical.choice` (checked by `proof/audit.py`, which has no
allowlist).
-/

open WebfoldModel WebfoldModel.Std Result

namespace EpubProofs

open EpubSpec

/-! ## Constructive helpers -/

private theorem mod_of_lt {a b : Nat} (h : a < b) : a % b = a := by
  cases a with
  | zero => rfl
  | succ a => exact if_neg (Nat.not_le_of_gt h)

private theorem toNat_ofNat_lt {w n : Nat} (h : n < 2 ^ w) : (BitVec.ofNat w n).toNat = n :=
  mod_of_lt h

private theorem ofNat_toNat_self {w : Nat} (x : BitVec w) : BitVec.ofNat w x.toNat = x :=
  BitVec.eq_of_toNat_eq (toNat_ofNat_lt x.isLt)

private theorem bind_ok {α β : Type} (x : α) (f : α → Result β) :
    (Bind.bind (Result.ok x) f : Result β) = f x := rfl

/-- Checked addition of two 64-bit words, with its value. -/
private theorem add64_ok (a b : Std.U64) (h : a.toNat + b.toNat < 2 ^ 64) :
    ∃ r : Std.U64, (a + b : Result Std.U64) = Result.ok r ∧ r.toNat = a.toNat + b.toNat := by
  refine ⟨BitVec.ofNat 64 (a.toNat + b.toNat), ?_, toNat_ofNat_lt h⟩
  show Std.checkedAdd a b = _
  unfold Std.checkedAdd
  exact if_pos h

/-- Checked subtraction of two 64-bit words, with its value. -/
private theorem sub64_ok (a b : Std.U64) (h : b.toNat ≤ a.toNat) :
    ∃ r : Std.U64, (a - b : Result Std.U64) = Result.ok r ∧ r.toNat = a.toNat - b.toNat := by
  refine ⟨BitVec.ofNat 64 (a.toNat - b.toNat), ?_,
    toNat_ofNat_lt (Nat.lt_of_le_of_lt (Nat.sub_le _ _) a.isLt)⟩
  show Std.checkedSub a b = _
  unfold Std.checkedSub
  exact if_pos h

/-- A BitVec equal to a literal is the Nat equality of the values. -/
private theorem bv_eq_of {w : Nat} (a : BitVec w) (n : Nat) (h : n < 2 ^ w) :
    a = BitVec.ofNat w n ↔ a.toNat = n :=
  ⟨fun e => by rw [e, toNat_ofNat_lt h],
    fun e => BitVec.eq_of_toNat_eq (by rw [toNat_ofNat_lt h]; exact e)⟩

/-- A BitVec `<` between two values is the Nat comparison of the values. -/
private theorem bv_lt_iff {w : Nat} (a b : BitVec w) : (a < b) ↔ a.toNat < b.toNat :=
  Iff.rfl

/-- A BitVec `≤` between two values is the Nat comparison of the values. -/
private theorem bv_le_iff {w : Nat} (a b : BitVec w) : (a ≤ b) ↔ a.toNat ≤ b.toNat :=
  Iff.rfl

/-- `decide` of equivalent propositions agree (a Bool equation, so no `propext`). -/
private theorem decide_congr' {p q : Prop} [Decidable p] [Decidable q] (h : p ↔ q) :
    decide p = decide q := by
  by_cases hp : p
  · rw [decide_eq_true hp, decide_eq_true (h.mp hp)]
  · rw [decide_eq_false hp, decide_eq_false (fun hq => hp (h.mpr hq))]

/-- `n - s + s = n` for `s ≤ n` (by induction; the core lemma uses `propext`). -/
private theorem sub_add_cancel' : ∀ (n s : Nat), s ≤ n → n - s + s = n
  | _, 0, _ => rfl
  | 0, _ + 1, h => absurd h (Nat.not_succ_le_zero _)
  | n + 1, s + 1, h => by
      rw [Nat.succ_sub_succ, ← Nat.add_assoc, sub_add_cancel' n s (Nat.le_of_succ_le_succ h)]

private theorem le_of_add_le_add_right' : ∀ {a b : Nat} (c : Nat), a + c ≤ b + c → a ≤ b
  | _, _, 0, h => h
  | _, _, c + 1, h => le_of_add_le_add_right' c (Nat.le_of_succ_le_succ h)

/-- `b ≤ c - a` exactly when `a + b ≤ c`, for `a ≤ c`. -/
private theorem le_sub_iff' (a b c : Nat) (hac : a ≤ c) : b ≤ c - a ↔ a + b ≤ c := by
  constructor
  · intro h
    have e : c - a + a = c := sub_add_cancel' c a hac
    have h2 : b + a ≤ c - a + a := Nat.add_le_add_right h a
    rw [e, Nat.add_comm] at h2
    exact h2
  · intro h
    apply le_of_add_le_add_right' a
    rw [sub_add_cancel' c a hac, Nat.add_comm]
    exact h

/-- A byte-valued result that the exhaustive table certifies equals the literal. -/
private theorem u8_result_of (r : Result Std.U8) (c : Nat)
    (h : (match r with
      | Result.ok v => decide (v.toNat = c)
      | Result.fail _ => false) = true) :
    r = Result.ok (BitVec.ofNat 8 c) := by
  cases r with
  | fail e => cases h
  | ok v =>
    have hv : v.toNat = c := of_decide_eq_true h
    rw [← hv, ofNat_toNat_self]

/-! ## `opf_child_role` -/

theorem opf_child_role_refines (p n : Std.U8) (b : Bool) :
    epub_math.opf_child_role p n b = Result.ok (BitVec.ofNat 8 (childRole p.toNat n.toNat b)) := by
  cases b with
  | false => unfold epub_math.opf_child_role childRole; rfl
  | true =>
    have h := EpubTables.opf_child_role_table ⟨p.toNat, p.isLt⟩ ⟨n.toNat, n.isLt⟩
    rw [ofNat_toNat_self p, ofNat_toNat_self n] at h
    exact u8_result_of _ _ h

/-! ## `href_byte_class` and `pct_hex` -/

theorem href_byte_class_refines (b : Std.U8) :
    epub_math.href_byte_class b = Result.ok (BitVec.ofNat 8 (byteClass b.toNat)) := by
  have h : (match epub_math.href_byte_class (BitVec.ofNat 8 b.toNat) with
      | Result.ok v => decide (v.toNat = byteClass b.toNat)
      | Result.fail _ => false) = true :=
    EpubTables.href_byte_class_table ⟨b.toNat, b.isLt⟩
  rw [ofNat_toNat_self b] at h
  exact u8_result_of _ _ h

theorem pct_hex_refines (b : Std.U8) :
    epub_math.pct_hex b = Result.ok ((hexFields b.toNat).1, BitVec.ofNat 8 (hexFields b.toNat).2) := by
  have h : (match epub_math.pct_hex (BitVec.ofNat 8 b.toNat) with
      | Result.ok (f, v) =>
        decide (f = (hexFields b.toNat).1) && decide (v.toNat = (hexFields b.toNat).2)
      | Result.fail _ => false) = true :=
    EpubTables.pct_hex_table ⟨b.toNat, b.isLt⟩
  rw [ofNat_toNat_self b] at h
  cases hres : epub_math.pct_hex b with
  | fail e => rw [hres] at h; cases h
  | ok r =>
    rw [hres] at h
    obtain ⟨f, v⟩ := r
    have hf : f = (hexFields b.toNat).1 := of_decide_eq_true (band_left h)
    have hv : v.toNat = (hexFields b.toNat).2 := of_decide_eq_true (band_right h)
    rw [← hf, ← hv, ofNat_toNat_self]

/-! ## `manifest_item_accept` and `manifest_index_step` -/

theorem manifest_item_accept_refines (hasId hasHref : Bool) (idLen : Std.U64) :
    epub_math.manifest_item_accept hasId hasHref idLen =
      Result.ok (itemAccept hasId hasHref idLen.toNat) := by
  unfold epub_math.manifest_item_accept itemAccept
  cases hasId <;> cases hasHref
  · rfl
  · rfl
  · rfl
  · have z : (BitVec.ofNat 64 0).toNat = 0 := toNat_ofNat_lt (by decide)
    have hiff : (BitVec.ofNat 64 0 < idLen) ↔ 0 < idLen.toNat :=
      (bv_lt_iff _ _).trans (by rw [z])
    exact congrArg Result.ok (decide_congr' hiff)

theorem manifest_index_step_refines (present : Bool) (prev k : Std.U32) :
    epub_math.manifest_index_step present prev k =
      Result.ok (BitVec.ofNat 32 (indexFields present prev.toNat k.toNat).1,
        (indexFields present prev.toNat k.toNat).2) := by
  cases present
  · show Result.ok (k, false) = Result.ok (BitVec.ofNat 32 k.toNat, false)
    rw [ofNat_toNat_self]
  · show Result.ok (prev, true) = Result.ok (BitVec.ofNat 32 prev.toNat, true)
    rw [ofNat_toNat_self]

/-! ## `spine_step` -/

theorem spine_step_refines (found dup : Bool) (index count : Std.U32) (idEq contentDoc seen : Bool) :
    epub_math.spine_step found dup index count idEq contentDoc seen =
      Result.ok (BitVec.ofNat 8 (spineCode found dup index.toNat count.toNat idEq contentDoc seen)) := by
  unfold epub_math.spine_step spineCode
  by_cases hlt : index < count
  · rw [if_pos hlt, decide_eq_true ((bv_lt_iff index count).1 hlt)]
    cases found <;> cases dup <;> cases idEq <;> cases contentDoc <;> cases seen <;> rfl
  · rw [if_neg hlt, decide_eq_false (fun h => hlt ((bv_lt_iff index count).2 h))]
    cases found <;> cases dup <;> cases idEq <;> cases contentDoc <;> cases seen <;> rfl

/-! ## `path_segment_kind` -/

theorem path_segment_kind_refines (len : Std.U64) (b0 b1 : Std.U8) :
    epub_math.path_segment_kind len b0 b1 =
      Result.ok (BitVec.ofNat 8 (segKindFields len.toNat b0.toNat b1.toNat)) := by
  unfold epub_math.path_segment_kind segKindFields
  by_cases hl1 : len.toNat = 1
  · have hl1' : len = 1#u64 := (bv_eq_of len 1 (by decide)).2 hl1
    rw [if_pos hl1']
    by_cases hb : b0.toNat = 46
    · have hb' : b0 = 46#u8 := (bv_eq_of b0 46 (by decide)).2 hb
      have hy : len.toNat = 1 ∧ b0.toNat = 46 := ⟨hl1, hb⟩
      rw [if_pos hb', if_pos hy]
    · have hb' : ¬ b0 = 46#u8 := fun e => hb ((bv_eq_of b0 46 (by decide)).1 e)
      have hl2 : ¬ len = 2#u64 := fun e => by
        have h2 := (bv_eq_of len 2 (by decide)).1 e
        rw [hl1] at h2
        exact absurd h2 (by decide)
      have hn : ¬ (len.toNat = 1 ∧ b0.toNat = 46) := fun h => hb h.2
      have hn2 : ¬ (len.toNat = 2 ∧ b0.toNat = 46 ∧ b1.toNat = 46) := fun h => by
        have h2 := h.1
        rw [hl1] at h2
        exact absurd h2 (by decide)
      rw [if_neg hb', if_neg hl2, if_neg hn, if_neg hn2]
  · have hl1' : ¬ len = 1#u64 := fun e => hl1 ((bv_eq_of len 1 (by decide)).1 e)
    have hn : ¬ (len.toNat = 1 ∧ b0.toNat = 46) := fun h => hl1 h.1
    rw [if_neg hl1', if_neg hn]
    by_cases hl2 : len.toNat = 2
    · have hl2' : len = 2#u64 := (bv_eq_of len 2 (by decide)).2 hl2
      rw [if_pos hl2']
      by_cases hb : b0.toNat = 46
      · have hb' : b0 = 46#u8 := (bv_eq_of b0 46 (by decide)).2 hb
        rw [if_pos hb']
        by_cases hb1 : b1.toNat = 46
        · have hb1' : b1 = 46#u8 := (bv_eq_of b1 46 (by decide)).2 hb1
          have hy : len.toNat = 2 ∧ b0.toNat = 46 ∧ b1.toNat = 46 := ⟨hl2, hb, hb1⟩
          rw [if_pos hb1', if_pos hy]
        · have hb1' : ¬ b1 = 46#u8 := fun e => hb1 ((bv_eq_of b1 46 (by decide)).1 e)
          have hn3 : ¬ (len.toNat = 2 ∧ b0.toNat = 46 ∧ b1.toNat = 46) := fun h => hb1 h.2.2
          rw [if_neg hb1', if_neg hn3]
      · have hb' : ¬ b0 = 46#u8 := fun e => hb ((bv_eq_of b0 46 (by decide)).1 e)
        have hn3 : ¬ (len.toNat = 2 ∧ b0.toNat = 46 ∧ b1.toNat = 46) := fun h => hb h.2.1
        rw [if_neg hb', if_neg hn3]
    · have hl2' : ¬ len = 2#u64 := fun e => hl2 ((bv_eq_of len 2 (by decide)).1 e)
      have hn3 : ¬ (len.toNat = 2 ∧ b0.toNat = 46 ∧ b1.toNat = 46) := fun h => hl2 h.1
      rw [if_neg hl2', if_neg hn3]

/-! ## `zip_entry_bound` -/

theorem zip_entry_bound_refines (declared read max : Std.U64) :
    epub_math.zip_entry_bound declared read max =
      Result.ok (entryBoundOk declared.toNat read.toNat max.toNat) := by
  unfold epub_math.zip_entry_bound entryBoundOk
  by_cases hd : declared.toNat ≤ max.toNat
  · rw [if_pos ((bv_le_iff declared max).2 hd), decide_eq_true hd]
    exact congrArg Result.ok (decide_congr' (bv_le_iff read max))
  · rw [if_neg (fun h => hd ((bv_le_iff declared max).1 h)), decide_eq_false hd]
    rfl

/-! ## `epub_text_budget` -/

theorem epub_text_budget_refines (out sep md max : Std.U64) :
    epub_math.epub_text_budget out sep md max =
      Result.ok ((textBudget out.toNat sep.toNat md.toNat max.toNat).1,
        BitVec.ofNat 64 (textBudget out.toNat sep.toNat md.toNat max.toNat).2) := by
  unfold epub_math.epub_text_budget textBudget
  by_cases h1 : out.toNat ≤ max.toNat
  · obtain ⟨room, hr, hrt⟩ := sub64_ok max out h1
    rw [if_pos ((bv_le_iff out max).2 h1), hr, bind_ok]
    by_cases h2 : sep.toNat ≤ room.toNat
    · obtain ⟨room2, hr2, hrt2⟩ := sub64_ok room sep h2
      rw [if_pos ((bv_le_iff sep room).2 h2), hr2, bind_ok]
      have h2' : sep.toNat ≤ max.toNat - out.toNat := by
        rw [← hrt]
        exact h2
      by_cases h3 : md.toNat ≤ room2.toNat
      · have h3' : md.toNat ≤ max.toNat - out.toNat - sep.toNat := by
          rw [← hrt, ← hrt2]
          exact h3
        have hA : sep.toNat + md.toNat ≤ max.toNat - out.toNat :=
          (le_sub_iff' sep.toNat md.toNat (max.toNat - out.toNat) h2').1 h3'
        have hB : out.toNat + (sep.toNat + md.toNat) ≤ max.toNat :=
          (le_sub_iff' out.toNat (sep.toNat + md.toNat) max.toNat h1).1 hA
        have hfit : out.toNat + sep.toNat + md.toNat ≤ max.toNat := by
          rw [Nat.add_assoc]
          exact hB
        have h4 : out.toNat + sep.toNat ≤ max.toNat :=
          (le_sub_iff' out.toNat sep.toNat max.toNat h1).1 h2'
        have hb1 : out.toNat + sep.toNat < 2 ^ 64 := Nat.lt_of_le_of_lt h4 max.isLt
        obtain ⟨s1, hs1, hs1t⟩ := add64_ok out sep hb1
        have hb2 : s1.toNat + md.toNat < 2 ^ 64 := by
          rw [hs1t]
          exact Nat.lt_of_le_of_lt hfit max.isLt
        obtain ⟨s2, hs2, hs2t⟩ := add64_ok s1 md hb2
        rw [if_pos ((bv_le_iff md room2).2 h3), if_pos hfit, hs1, bind_ok, hs2, bind_ok]
        have e : BitVec.ofNat 64 (out.toNat + sep.toNat + md.toNat) = s2 :=
          BitVec.eq_of_toNat_eq (by
            rw [toNat_ofNat_lt (Nat.lt_of_le_of_lt hfit max.isLt), hs2t, hs1t])
        show Result.ok (true, s2) =
          Result.ok (true, BitVec.ofNat 64 (out.toNat + sep.toNat + md.toNat))
        rw [e]
      · have hfalse : ¬ (out.toNat + sep.toNat + md.toNat ≤ max.toNat) := by
          intro hf
          apply h3
          rw [hrt2, hrt]
          exact (le_sub_iff' sep.toNat md.toNat (max.toNat - out.toNat) h2').2
            ((le_sub_iff' out.toNat (sep.toNat + md.toNat) max.toNat h1).2
              (by rw [← Nat.add_assoc]; exact hf))
        rw [if_neg (fun e => h3 ((bv_le_iff md room2).1 e)), if_neg hfalse]
        show Result.ok (false, out) = Result.ok (false, BitVec.ofNat 64 out.toNat)
        rw [ofNat_toNat_self]
    · have hfalse : ¬ (out.toNat + sep.toNat + md.toNat ≤ max.toNat) := by
        intro hf
        apply h2
        rw [hrt]
        exact (le_sub_iff' out.toNat sep.toNat max.toNat h1).2
          (Nat.le_trans (Nat.le_add_right _ _) hf)
      rw [if_neg (fun e => h2 ((bv_le_iff sep room).1 e)), if_neg hfalse]
      show Result.ok (false, out) = Result.ok (false, BitVec.ofNat 64 out.toNat)
      rw [ofNat_toNat_self]
  · rw [if_neg (fun e => h1 ((bv_le_iff out max).1 e)),
      if_neg (fun hf => h1 (Nat.le_trans (Nat.le_trans (Nat.le_add_right _ _)
        (Nat.le_add_right _ _)) hf))]
    show Result.ok (false, out) = Result.ok (false, BitVec.ofNat 64 out.toNat)
    rw [ofNat_toNat_self]

/-! ## Contracts production relies on (kernel level)

Each theorem reads a kernel result that production branches on and states what that result
guarantees, as a consequence of the refinement above and of the specification. -/

private theorem childRole_lt (p n : Nat) (b : Bool) : childRole p n b < 256 := by
  unfold childRole
  repeat' split
  all_goals decide

private theorem spineCode_lt (found dup : Bool) (index count : Nat) (idEq contentDoc seen : Bool) :
    spineCode found dup index count idEq contentDoc seen < 256 := by
  unfold spineCode
  repeat' split
  all_goals decide

private theorem ofNat8_eq_iff (c : Nat) (hc : c < 2 ^ 8) (k : Nat) (hk : k < 2 ^ 8) :
    BitVec.ofNat 8 c = BitVec.ofNat 8 k → c = k := by
  intro e
  rw [← toNat_ofNat_lt hc, e, toNat_ofNat_lt hk]

private theorem segKindFields_lt (len b0 b1 : Nat) : segKindFields len b0 b1 < 256 := by
  unfold segKindFields
  repeat' split
  all_goals decide

/-- A manifest item (`opf_child_role` returns `4`) is only an OPF `item` directly inside the
manifest: the kernel never classifies anything else as a manifest entry. -/
theorem opf_child_role_item (p n : Std.U8) (b : Bool)
    (h : epub_math.opf_child_role p n b = Result.ok 4#u8) :
    b = true ∧ p.toNat = 2 ∧ n.toNat = 4 := by
  rw [opf_child_role_refines] at h
  have e := Result.ok.inj h
  have hc : childRole p.toNat n.toNat b = 4 :=
    ofNat8_eq_iff _ (childRole_lt _ _ _) 4 (by decide) e
  exact childRole_item hc

/-- A spine reference (`opf_child_role` returns `5`) is only an OPF `itemref` directly inside
the spine. -/
theorem opf_child_role_itemref (p n : Std.U8) (b : Bool)
    (h : epub_math.opf_child_role p n b = Result.ok 5#u8) :
    b = true ∧ p.toNat = 3 ∧ n.toNat = 5 := by
  rw [opf_child_role_refines] at h
  have e := Result.ok.inj h
  have hc : childRole p.toNat n.toNat b = 5 :=
    ofNat8_eq_iff _ (childRole_lt _ _ _) 5 (by decide) e
  exact childRole_itemref hc

/-- An accepted manifest item has its required `id` and `href` and a non-empty id. -/
theorem manifest_item_accept_spec (hasId hasHref : Bool) (idLen : Std.U64)
    (h : epub_math.manifest_item_accept hasId hasHref idLen = Result.ok true) :
    hasId = true ∧ hasHref = true ∧ 0 < idLen.toNat := by
  rw [manifest_item_accept_refines] at h
  exact itemAccept_spec (Result.ok.inj h)

/-- The kernel reads a spine entry (`spine_step` returns `0`) only for a found, unduplicated,
in-range entry whose stored id is the idref, a content document, and not read before. -/
theorem spine_step_read_sound (found dup : Bool) (index count : Std.U32)
    (idEq contentDoc seen : Bool)
    (h : epub_math.spine_step found dup index count idEq contentDoc seen = Result.ok 0#u8) :
    found = true ∧ dup = false ∧ index.toNat < count.toNat ∧ idEq = true ∧
      contentDoc = true ∧ seen = false := by
  rw [spine_step_refines] at h
  have hc : spineCode found dup index.toNat count.toNat idEq contentDoc seen = 0 :=
    ofNat8_eq_iff _ (spineCode_lt _ _ _ _ _ _ _) 0 (by decide) (Result.ok.inj h)
  exact spineCode_read hc

/-- An entry is accepted by `zip_entry_bound` only when its declared size and the bytes read
are both within the bound. -/
theorem zip_entry_bound_spec (declared read max : Std.U64)
    (h : epub_math.zip_entry_bound declared read max = Result.ok true) :
    declared.toNat ≤ max.toNat ∧ read.toNat ≤ max.toNat := by
  rw [zip_entry_bound_refines] at h
  exact entryBoundOk_spec (Result.ok.inj h)

/-- A chapter is appended by `epub_text_budget` only when the joined text, with its separator,
is within the budget; the returned length is exactly that total. -/
theorem epub_text_budget_accept (out sep md max : Std.U64) (v : Std.U64)
    (h : epub_math.epub_text_budget out sep md max = Result.ok (true, v)) :
    v.toNat = out.toNat + sep.toNat + md.toNat ∧ out.toNat + sep.toNat + md.toNat ≤ max.toNat := by
  rw [epub_text_budget_refines] at h
  have e := Prod.mk.inj (Result.ok.inj h)
  have hacc : (textBudget out.toNat sep.toNat md.toNat max.toNat).1 = true := e.1
  have hv : BitVec.ofNat 64 (textBudget out.toNat sep.toNat md.toNat max.toNat).2 = v := e.2
  have hspec := textBudget_accept hacc
  refine ⟨?_, hspec.2⟩
  rw [← hv, hspec.1]
  exact toNat_ofNat_lt (Nat.lt_of_le_of_lt hspec.2 max.isLt)

/-- A chapter is refused by `epub_text_budget` only when the joined text would exceed the
budget. -/
theorem epub_text_budget_refuse (out sep md max : Std.U64) (v : Std.U64)
    (h : epub_math.epub_text_budget out sep md max = Result.ok (false, v)) :
    max.toNat < out.toNat + sep.toNat + md.toNat := by
  rw [epub_text_budget_refines] at h
  have e := Prod.mk.inj (Result.ok.inj h)
  exact textBudget_refuse e.1

/-- A hex digit's value is below sixteen (the `pct_hex` kernel's accepted value). -/
theorem pct_hex_value_lt (b : Std.U8) (v : Std.U8)
    (h : epub_math.pct_hex b = Result.ok (true, v)) : v.toNat < 16 := by
  rw [pct_hex_refines] at h
  have e := Prod.mk.inj (Result.ok.inj h)
  have hf : (hexFields b.toNat).1 = true := e.1
  have hv : BitVec.ofNat 8 (hexFields b.toNat).2 = v := e.2
  have hl : hexVal b.toNat = some (hexFields b.toNat).2 := hexFields_spec b.toNat hf
  have hlt := hexVal_lt hl
  rw [← hv, toNat_ofNat_lt (Nat.lt_trans hlt (by decide))]
  exact hlt

/-- `path_segment_kind` returns `1` only for the segment `.`, and `2` only for `..`. -/
theorem path_segment_kind_one (len : Std.U64) (b0 b1 : Std.U8)
    (h : epub_math.path_segment_kind len b0 b1 = Result.ok 1#u8) :
    len.toNat = 1 ∧ b0.toNat = 46 := by
  rw [path_segment_kind_refines] at h
  have hc : segKindFields len.toNat b0.toNat b1.toNat = 1 :=
    ofNat8_eq_iff _ (segKindFields_lt _ _ _) 1 (by decide) (Result.ok.inj h)
  unfold segKindFields at hc
  by_cases h1 : len.toNat = 1 ∧ b0.toNat = 46
  · exact h1
  · rw [if_neg h1] at hc
    by_cases h2 : len.toNat = 2 ∧ b0.toNat = 46 ∧ b1.toNat = 46
    · rw [if_pos h2] at hc
      exact absurd hc (by decide)
    · rw [if_neg h2] at hc
      exact absurd hc (by decide)

end EpubProofs
