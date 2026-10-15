import EpubSpec
import EpubMath

/-!
Exhaustive kernel checks for the byte-valued kernels of `proof/extract_epub.rs` (`epub_math`).

Each statement quantifies over every input of the kernel (every pair of bytes for
`opf_child_role`, every byte for `href_byte_class` and `pct_hex`) and is decided by the kernel
itself (`decide +kernel`), so no input is left to a case split. The statements compare the
extracted `Result` function with the field functions of `EpubSpec`. They rest on no assumption
beyond the kernel's evaluation of `Decidable` instances (checked by `proof/audit.py`).
-/

open WebfoldModel WebfoldModel.Std Result

namespace EpubTables

open EpubSpec

/-- `opf_child_role` on every pair of bytes, for a manifest/spine element (`opf_ns = true`). -/
theorem opf_child_role_table : ∀ (i j : Fin 256),
    (match epub_math.opf_child_role (BitVec.ofNat 8 i.val) (BitVec.ofNat 8 j.val) true with
      | Result.ok v => decide (v.toNat = childRole i.val j.val true)
      | Result.fail _ => false) = true := by
  decide +kernel

/-- `href_byte_class` on every byte. -/
theorem href_byte_class_table : ∀ (i : Fin 256),
    (match epub_math.href_byte_class (BitVec.ofNat 8 i.val) with
      | Result.ok v => decide (v.toNat = byteClass i.val)
      | Result.fail _ => false) = true := by
  decide +kernel

/-- `pct_hex` on every byte: the flag and the value both agree with `hexFields`. -/
theorem pct_hex_table : ∀ (i : Fin 256),
    (match epub_math.pct_hex (BitVec.ofNat 8 i.val) with
      | Result.ok (f, v) => decide (f = (hexFields i.val).1) && decide (v.toNat = (hexFields i.val).2)
      | Result.fail _ => false) = true := by
  decide +kernel

end EpubTables
