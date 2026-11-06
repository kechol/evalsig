//! The referee (D-014, D-029): its home and authority, cutting and excluding holdout
//! items, gate conditions and their approval, sealing a holdout, judging a candidate under approved conditions with the
//! Ladder, and `gate serve`. Everything the threat model (SPEC §8.0) protects is here.

pub mod approve;
pub mod conditions;
pub mod exclude;
pub mod gate;
pub mod holdout;
pub mod home;
pub mod seal;
pub mod serve;
