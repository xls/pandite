// The error type of the installer.

/**
 * An error with a message for the user. The installer prints the message
 * without a stack and exits with `exitCode` (1, or 2 for a usage error).
 */
export class InstallerError extends Error {
  constructor(message, { exitCode = 1 } = {}) {
    super(message);
    this.name = 'InstallerError';
    this.exitCode = exitCode;
  }
}

/** A usage error: a wrong command line. Exit code 2. */
export function usageError(message) {
  return new InstallerError(`${message}. Run "npx pandite --help" for the options.`, { exitCode: 2 });
}
