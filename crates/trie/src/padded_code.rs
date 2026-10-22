use sbv_primitives::{Bytes, types::revm::Bytecode};
use std::fmt;

/// Code backed by an aligned buffer with validated zero padding for REVM execution.
///
/// The original bytes determine the code hash and all execution-visible code lengths.
/// Padding is only an execution detail. Static guest input can be passed as
/// [`Bytes::from_static`] to retain its allocation throughout bytecode analysis.
/// Jump tables are always computed locally by REVM, never supplied by the witness.
#[derive(Clone, Debug)]
pub struct PaddedCode {
    padded: Bytes,
    original_len: usize,
}

/// Invalid input to [`PaddedCode::new`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaddedCodeError {
    /// The original length plus the minimum padding overflows `usize`.
    LengthOverflow,
    /// The buffer does not contain the original bytes and minimum padding.
    InsufficientPadding,
    /// The first code byte is not aligned to [`PaddedCode::ALIGNMENT`].
    Misaligned,
    /// At least one byte after the original code is nonzero.
    NonzeroPadding,
    /// Code with the EIP-7702 prefix does not contain a valid delegation designation.
    InvalidEip7702,
}

impl fmt::Display for PaddedCodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LengthOverflow => "original code length plus padding overflows usize",
            Self::InsufficientPadding => "code requires at least 33 bytes of padding",
            Self::Misaligned => "code buffer must start at a 4-byte aligned address",
            Self::NonzeroPadding => "code padding must contain only zero bytes",
            Self::InvalidEip7702 => "invalid EIP-7702 delegation designation",
        })
    }
}

impl std::error::Error for PaddedCodeError {}

impl PaddedCode {
    /// Maximum PUSH immediate length plus a terminating STOP instruction.
    pub const MIN_PADDING: usize = 33;

    /// Alignment required by the borrowed guest code buffer.
    pub const ALIGNMENT: usize = 4;

    /// Validates a padded buffer without copying its code payload.
    ///
    /// `padded[..original_len]` is the original code, including any trailing zeros.
    /// The remaining bytes must contain at least [`Self::MIN_PADDING`] zeros.
    /// EIP-7702 designations use the same envelope, with exactly 23 original bytes.
    pub fn new(padded: Bytes, original_len: usize) -> Result<Self, PaddedCodeError> {
        let minimum_len = original_len
            .checked_add(Self::MIN_PADDING)
            .ok_or(PaddedCodeError::LengthOverflow)?;
        if padded.len() < minimum_len {
            return Err(PaddedCodeError::InsufficientPadding);
        }
        if !(padded.as_ptr() as usize).is_multiple_of(Self::ALIGNMENT) {
            return Err(PaddedCodeError::Misaligned);
        }
        if padded[original_len..].iter().any(|byte| *byte != 0) {
            return Err(PaddedCodeError::NonzeroPadding);
        }
        let code = Self {
            padded,
            original_len,
        };
        if code.is_eip7702() {
            Bytecode::new_eip7702_raw(code.original_bytes())
                .map_err(|_| PaddedCodeError::InvalidEip7702)?;
        }
        Ok(code)
    }

    /// Returns the original bytes as a shared view of the input allocation.
    pub fn original_bytes(&self) -> Bytes {
        self.padded.slice(..self.original_len)
    }

    /// Returns the original code without its execution padding.
    pub fn original_byte_slice(&self) -> &[u8] {
        &self.padded[..self.original_len]
    }

    /// Returns the original code length, excluding execution padding.
    pub const fn original_len(&self) -> usize {
        self.original_len
    }

    /// Returns the validated execution buffer, including its zero padding.
    pub const fn padded_bytes(&self) -> &Bytes {
        &self.padded
    }

    /// Analyzes the code using REVM while retaining the existing payload allocation.
    ///
    /// REVM builds the jump table over the padded buffer. Thirty-three trailing zeros
    /// cover any truncated PUSH32 immediate and its terminating STOP, so REVM does
    /// not need to allocate a replacement code buffer. Extra zeros also cover the
    /// one-byte immediates of DUPN, SWAPN and EXCHANGE. The original length is then
    /// restored with REVM's public analyzed-bytecode constructor. No jump destination
    /// can be introduced by the padding because all padding bytes are STOPs.
    pub fn to_bytecode(&self) -> Bytecode {
        if self.is_eip7702() {
            return Bytecode::new_eip7702_raw(self.original_bytes())
                .expect("EIP-7702 designation was validated at construction");
        }
        let analyzed = Bytecode::new_legacy(self.padded.clone());
        Bytecode::new_analyzed(
            analyzed.bytes(),
            self.original_len,
            analyzed
                .legacy_jump_table()
                .expect("new_legacy always produces a legacy jump table")
                .clone(),
        )
    }

    fn is_eip7702(&self) -> bool {
        self.original_byte_slice().starts_with(&[0xef, 0x01])
    }
}
