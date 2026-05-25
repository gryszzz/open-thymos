import { siteConfig } from "@/lib/site";

type ThymosLogoProps = {
  className?: string;
  wordmark?: boolean;
  priority?: boolean;
};

export function ThymosLogo({
  className,
  wordmark = true,
  priority = false,
}: ThymosLogoProps) {
  return (
    <div className={className ? `thymos-logo ${className}` : "thymos-logo"}>
      <img
        className="thymos-mark"
        src={`${siteConfig.basePath}/thymos-mark.png`}
        alt=""
        aria-hidden="true"
        width={1024}
        height={1024}
        decoding="async"
        loading="eager"
        fetchPriority={priority ? "high" : "auto"}
      />

      {wordmark ? (
        <span className="thymos-wordmark">
          <strong>THYMOS</strong>
          <span>execution substrate</span>
        </span>
      ) : null}
    </div>
  );
}
