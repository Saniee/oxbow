// Minimal subset of the built-in Git extension API (extensions/git/src/api/git.d.ts in vscode).
import type { Event, Uri } from 'vscode';

export interface Branch {
  readonly name?: string;
  readonly commit?: string;
  readonly upstream?: { readonly name: string; readonly remote: string };
}

export interface RepositoryState {
  readonly HEAD: Branch | undefined;
  readonly onDidChange: Event<void>;
}

export const enum ForcePushMode {
  Force = 0,
  ForceWithLease = 1,
}

export interface FetchOptions {
  remote?: string;
  ref?: string;
  all?: boolean;
  prune?: boolean;
}

export interface Repository {
  readonly rootUri: Uri;
  readonly state: RepositoryState;
  status(): Promise<void>;
  // Network operations go through the built-in extension so its credential prompts (askpass) apply.
  fetch(options?: FetchOptions): Promise<void>;
  /** Runs `git push [flags] <remoteName> <branchName>`; branchName can be any refspec. */
  pull(): Promise<void>;
  push(remoteName?: string, branchName?: string, setUpstream?: boolean, force?: ForcePushMode): Promise<void>;
  /** Commits like the Source Control view's Commit button, so hooks, signing and git.postCommitCommand apply. */
  commit(message: string, opts?: CommitOptions): Promise<void>;
  /** The Source Control view's commit message box. */
  readonly inputBox: { value: string };
}

export interface CommitOptions {
  amend?: boolean;
}

export interface API {
  readonly git: { readonly path: string };
  readonly repositories: Repository[];
  readonly onDidOpenRepository: Event<Repository>;
  readonly onDidCloseRepository: Event<Repository>;
  toGitUri(uri: Uri, ref: string): Uri;
  getRepository(uri: Uri): Repository | null;
}

export interface GitExtension {
  readonly enabled: boolean;
  getAPI(version: 1): API;
}
