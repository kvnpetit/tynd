export interface OAuthStartArgs {
  port?: number | null;
  path?: string | null;
  successHtml?: string | null;
  expectedState?: string | null;
}

export interface OAuthStartResult {
  id: number;
  port: number;
  redirectUri: string;
}

export interface OAuthIdArgs {
  id: number;
}

export interface OAuthCallbackEvent {
  id: number;
  params: Record<string, string>;
}

export interface OAuthErrorEvent {
  id: number;
  error: string;
}

export type OAuthEventMap = {
  "oauth:callback": OAuthCallbackEvent;
  "oauth:error": OAuthErrorEvent;
};
