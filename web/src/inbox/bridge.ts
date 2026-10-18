export type InboxActions = {
  assignOpen: boolean;
  onAssignKey: (event: KeyboardEvent) => void;
  markReviewed: () => void;
  dismiss: () => void;
  openAssign: () => void;
  focusFilter: () => void;
};

export const inboxBridge: { current: InboxActions | null } = { current: null };
