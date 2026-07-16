/**
 * The typed error surfaced to applications (RUNTIME-API §9).
 *
 * The host scrubs every failure to one of these codes before it crosses the
 * boundary — messages never carry credentials, host paths or vault internals.
 */

/** The closed set of error codes an application may observe (RUNTIME-API §9). */
export const ERROR_CODES = [
  "PERMISSION_DENIED",
  "QUOTA_EXCEEDED",
  "NOT_AVAILABLE",
  "CONNECTION_REQUIRED",
  "INVALID_ARGUMENT",
  "INTERNAL",
] as const;

/** One of the §9 error codes. */
export type PoidErrorCode = (typeof ERROR_CODES)[number];

/** The error every rejected `poid.*` call throws. */
export class PoidError extends Error {
  /** Machine-readable code from {@link ERROR_CODES}. */
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = "PoidError";
    this.code = code;
  }
}
