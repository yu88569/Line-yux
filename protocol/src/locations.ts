export type LocationInfo = {
  title: string;
  address: string;
  latitude: number;
  longitude: number;
};

function coordinate(value: unknown): number | null {
  if (typeof value === "number") return value;
  if (typeof value !== "string") return null;
  const decimal = value.trim();
  if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)$/.test(decimal)) return null;
  return Number(decimal);
}

export function parseLocation(value: unknown): LocationInfo | null {
  if (!value || typeof value !== "object") return null;
  if (!("latitude" in value) || !("longitude" in value)) return null;
  const latitude = coordinate(value.latitude);
  const longitude = coordinate(value.longitude);
  if (
    latitude === null || longitude === null ||
    !Number.isFinite(latitude) || !Number.isFinite(longitude) ||
    Math.abs(latitude) > 90 || Math.abs(longitude) > 180
  ) return null;
  return {
    title: "title" in value && typeof value.title === "string"
      ? value.title.trim()
      : "",
    address: "address" in value && typeof value.address === "string"
      ? value.address.trim()
      : "",
    latitude,
    longitude,
  };
}

export const LOCATION_METADATA_VERSION = 2;

export function needsLocationMetadataRefresh(
  messages: Record<string, unknown>[],
): boolean {
  return messages.some((m) =>
    m.contentType === "LOCATION" &&
    m.locationMetadataVersion !== LOCATION_METADATA_VERSION
  );
}

export function locationPayload(
  raw: { location?: unknown },
  contentType: string,
) {
  return {
    location: contentType === "LOCATION" ? parseLocation(raw.location) : null,
    locationMetadataVersion: contentType === "LOCATION"
      ? LOCATION_METADATA_VERSION
      : null,
  };
}
