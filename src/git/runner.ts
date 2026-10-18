import { execFile } from 'node:child_process';
import * as fs from 'node:fs';
import * as path from 'node:path';

export class GitError extends Error {
  constructor(
    message: string,
    readonly stderr: string,
    readonly exitCode: number,
    readonly args: readonly string[],
  ) {
    super(message);
  }
}

export interface ExecOptions {
  env?: Record<string, string>;
  /** Exit codes that resolve instead of throwing. */
  allowedExitCodes?: number[];
  /** Written to stdin; otherwise stdin is closed. */
  input?: string;
}

export interface ExecResult {
  stdout: string;
  stderr: string;
  exitCode: number;
}

export interface GitDirs {
  /** This working tree's git directory: HEAD, index, rebase-merge, ... */
  gitDir: string;
  /** The directory shared by all worktrees: refs, config, ... Same as gitDir outside linked worktrees. */
  commonDir: string;
}

/** A finished git command, as told to onGitCommand listeners. */
export interface GitCommandEvent {
  cwd: string;
  args: readonly string[];
  exitCode: number;
}

const commandListeners = new Set<(event: GitCommandEvent) => void>();

/** Calls `listener` after every git command any Git runs (the SlothGit tab's command log). Returns an unsubscribe. */
export function onGitCommand(listener: (event: GitCommandEvent) => void): () => void {
  commandListeners.add(listener);
  return () => commandListeners.delete(listener);
}

/** Per working tree folder, so looking up files in the git directory doesn't run git each time. */
const gitDirs = new Map<string, Promise<GitDirs>>();

/** Thin wrapper around the git CLI, bound to one repository. Has no vscode dependency so it can be tested with plain node. */
export class Git {
  constructor(
    readonly gitPath: string,
    readonly cwd: string,
  ) {}

  exec(args: string[], options: ExecOptions = {}): Promise<ExecResult> {
    return new Promise((resolve, reject) => {
      const child = execFile(
        this.gitPath,
        args,
        {
          cwd: this.cwd,
          env: { ...process.env, ...options.env },
          maxBuffer: 64 * 1024 * 1024,
          windowsHide: true,
        },
        (error, stdout, stderr) => {
          const exitCode = error ? (typeof error.code === 'number' ? error.code : -1) : 0;
          for (const listener of commandListeners) {
            try {
              listener({ cwd: this.cwd, args, exitCode });
            } catch {
              // A broken log must not break git.
            }
          }
          if (error && !options.allowedExitCodes?.includes(exitCode)) {
            const detail = (stderr || stdout).trim() || error.message;
            reject(new GitError(detail, stderr, exitCode, args));
          } else {
            resolve({ stdout, stderr, exitCode });
          }
        },
      );
      child.stdin?.end(options.input);
    });
  }

  async run(args: string[], options?: ExecOptions): Promise<string> {
    return (await this.exec(args, options)).stdout;
  }

  /**
   * Path of `name` in the git directory, as `git rev-parse --git-path` gives it for per-worktree files
   * (rebase-merge, MERGE_HEAD, BISECT_START, scratch files). Not for shared ones like refs or objects.
   */
  async gitDirPath(name: string): Promise<string> {
    return path.join((await this.dirs()).gitDir, name);
  }

  async dirs(): Promise<GitDirs> {
    let dirs = gitDirs.get(this.cwd);
    if (!dirs || !fs.existsSync((await dirs.catch(() => undefined))?.gitDir ?? '')) {
      dirs = this.run(['rev-parse', '--absolute-git-dir', '--git-common-dir']).then((out) => {
        const [gitDir, commonDir] = out.trim().split(/\r?\n/);
        return { gitDir, commonDir: path.resolve(this.cwd, commonDir) };
      });
      gitDirs.set(this.cwd, dirs);
      dirs.catch(() => gitDirs.delete(this.cwd));
    }
    return dirs;
  }
}
