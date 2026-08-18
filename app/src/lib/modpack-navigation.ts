import { validateModpackPointerUrl } from "./modpack-ui";

export function managedModpackInstallRoute(pointerUrl: string): string {
  const searchParams = new URLSearchParams();
  searchParams.set("tab", "modpack");
  searchParams.set("pointer", pointerUrl);
  return `/download?${searchParams.toString()}`;
}

export function managedModpackPointerFromSearch(
  searchParams: URLSearchParams,
): string | null {
  const pointerUrl = searchParams.get("pointer");
  if (pointerUrl === null) return null;

  const trimmed = pointerUrl.trim();
  return validateModpackPointerUrl(trimmed) === null ? trimmed : null;
}
