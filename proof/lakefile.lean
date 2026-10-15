import Lake
open Lake DSL

-- Self-contained: `Constructive` is the extraction prelude (no Aeneas or Mathlib
-- dependency, so Lake downloads nothing). `PdfSpec` is pure Lean over Nat/Bool/List.
package superdocumentsProof where
  leanOptions := #[
    { name := `autoImplicit, value := false },
    { name := `warningAsError, value := true }
  ]

@[default_target]
lean_lib PdfProof where
  roots := #[`Constructive, `PdfSpec, `PdfMath, `PdfProofs]
