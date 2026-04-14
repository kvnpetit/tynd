import path from "path";
import fs from "fs";

/**
 * Simple persistent JSON key-value store, scoped to an app.
 * Writes are atomic (tmp file + rename), reads are synchronous and cached.
 */
export class KeyValueStore {
  private data: Record<string, unknown> = {};
  private loaded = false;

  constructor(private readonly filePath: string) {}

  private load(): void {
    if (this.loaded) return;
    this.loaded = true;
    try {
      const raw = fs.readFileSync(this.filePath, "utf-8");
      this.data = JSON.parse(raw);
    } catch { /* file missing or corrupt — start fresh */ }
  }

  private flush(): void {
    try {
      fs.mkdirSync(path.dirname(this.filePath), { recursive: true });
      const tmp = `${this.filePath}.tmp`;
      fs.writeFileSync(tmp, JSON.stringify(this.data, null, 2));
      fs.renameSync(tmp, this.filePath);
    } catch (err) {
      console.error("[bunview] store flush failed:", err);
    }
  }

  get<T = unknown>(key: string): T | undefined {
    this.load();
    return this.data[key] as T | undefined;
  }

  set(key: string, value: unknown): void {
    this.load();
    this.data[key] = value;
    this.flush();
  }

  delete(key: string): void {
    this.load();
    delete this.data[key];
    this.flush();
  }

  has(key: string): boolean {
    this.load();
    return key in this.data;
  }

  clear(): void {
    this.data = {};
    this.flush();
  }

  keys(): string[] {
    this.load();
    return Object.keys(this.data);
  }

  entries<T = unknown>(): [string, T][] {
    this.load();
    return Object.entries(this.data) as [string, T][];
  }
}
