export const Role = {
  Generic: 0,
  Button: 1,
  Text: 2,
  Image: 3,
  Scroll: 4,
  Switch: 5,
  TextField: 6,
} as const;

export type Role = (typeof Role)[keyof typeof Role];
