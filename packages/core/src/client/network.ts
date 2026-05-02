import { osCall } from "./_internal";

/**
 * Network interface descriptor returned by {@link network.interfaces}.
 *
 * `mac` and `is_up` are surfaced as `null` when the underlying crate
 * (`if-addrs` 0.13) cannot report them on the current platform — the
 * shape stays stable across OSes so frontend code doesn't branch.
 */
export interface NetworkInterface {
  name: string;
  /** Every IPv4 / IPv6 address bound to the interface, in discovery order. */
  ips: string[];
  /** Hardware MAC address, or `null` when the host can't read it. */
  mac: string | null;
  /** Up/down status — `null` when the host can't read it. */
  is_up: boolean | null;
  /** Whether this is a loopback interface (e.g. `lo` / `lo0`). */
  is_loopback: boolean;
}

/**
 * `network` — enumerate the host's network interfaces.
 *
 * Backed by `if-addrs` (`getifaddrs` on Unix, `GetAdaptersAddresses` on
 * Windows). The raw call returns one entry per (interface, IP) pair; the
 * host collapses them so each interface appears once with all its IPs
 * grouped together.
 */
export const network = {
  /** Snapshot of every interface with its assigned IPs. */
  interfaces(): Promise<NetworkInterface[]> {
    return osCall("network", "interfaces", {});
  },
};
