export interface TrayMenuItem {
  id: string;
  label: string;
  checked?: boolean;
  enabled?: boolean;
  separator?: boolean;
}

export interface MenuItem {
  id?: string;
  label: string;
  accelerator?: string;      // e.g. "CmdOrCtrl+S"
  checked?: boolean;
  enabled?: boolean;
  separator?: boolean;
  submenu?: MenuItem[];
}
