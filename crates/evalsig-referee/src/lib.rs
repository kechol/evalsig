//! The referee (D-014, D-029): its home and authority, gate conditions and their
//! approval, sealing a holdout, judging a candidate under approved conditions with the
//! Ladder, and `gate serve`. Everything the threat model (SPEC §8.0) protects is here.

pub mod approve;
pub mod conditions;
pub mod gate;
pub mod home;
pub mod seal;
pub mod serve;
