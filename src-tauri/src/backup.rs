// ============================================================
//  backup.rs — backup/restore orchestration (BE-BACKUP owns)
// ============================================================
//
// Placeholder. BE-OAUTH declares the `mod backup;` line in lib.rs (it owns the
// four backend `mod` declarations so later cards never touch that hotspot).
// Gather 4 sources → manifest → crypto::encode → drive::upload, and the
// fail-safe restore (download → decode → pre-restore backup → write real
// paths) land in the BE-BACKUP card per docs/tsd/gdrive-backup-v1.md §4,§5.
