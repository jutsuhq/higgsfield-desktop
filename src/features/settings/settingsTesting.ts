/** A fake command that fails the way Rust's `AppError::Internal` does, for the settings tests. */
export const internalError = (message: string) => () => {
  throw Object.assign(new Error(message), { kind: "internal" });
};
