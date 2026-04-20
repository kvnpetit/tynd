import type { ComponentProps } from "react";

type Props = ComponentProps<"div"> & {
  size?: "default" | "narrow" | "wide";
};

const SIZES = {
  narrow: "max-w-3xl",
  default: "max-w-6xl",
  wide: "max-w-7xl",
} as const;

export function Container({
  size = "default",
  className = "",
  ...rest
}: Props) {
  return (
    <div className={`${SIZES[size]} mx-auto px-6 ${className}`} {...rest} />
  );
}
