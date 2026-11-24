#version 330 core

in vec2 uv;
out vec4 color;

uniform sampler2D picture;
uniform int projection;
uniform vec3 screen; // width, height and distance in meters
uniform vec4 rotation;
uniform vec3 origin;
uniform vec4 tangents; // left, right, down, up
uniform vec4 region; // UV origin and extent of this eye's image
uniform bool linear_target;

const float PI = 3.141592653589793;

vec3 rotate(vec3 direction) {
    vec3 twice_cross = 2.0 * cross(rotation.xyz, direction);
    return direction + rotation.w * twice_cross + cross(rotation.xyz, twice_cross);
}

vec3 sample_eye(vec2 coord) {
    vec2 half_texel = 0.5 / vec2(textureSize(picture, 0));
    vec2 first = region.xy + half_texel;
    vec2 last = region.xy + region.zw - half_texel;
    vec2 packed_uv = region.xy + coord * region.zw;
    float y = clamp(packed_uv.y, first.y, last.y);

    // Interpolate across the longitude seam inside this eye's tile.
    if (projection == 2 && (packed_uv.x < first.x || packed_uv.x > last.x)) {
        float x = packed_uv.x < first.x ? packed_uv.x + region.z : packed_uv.x;
        float blend = clamp((x - last.x) / (2.0 * half_texel.x), 0.0, 1.0);
        return mix(texture(picture, vec2(last.x, y)).rgb,
                   texture(picture, vec2(first.x, y)).rgb, blend);
    }
    return texture(picture, vec2(clamp(packed_uv.x, first.x, last.x), y)).rgb;
}

vec3 srgb_to_linear(vec3 rgb) {
    return mix(pow((rgb + 0.055) / 1.055, vec3(2.4)), rgb / 12.92,
               lessThanEqual(rgb, vec3(0.04045)));
}

void main() {
    vec3 ray = normalize(rotate(vec3(mix(tangents.x, tangents.y, uv.x),
                                    mix(tangents.z, tangents.w, uv.y), -1.0)));
    vec2 coord;
    color = vec4(0.0, 0.0, 0.0, 1.0);
    if (projection == 0) {
        if (ray.z >= -0.00001 || origin.z <= -screen.z) {
            return;
        }
        float distance_to_plane = (-screen.z - origin.z) / ray.z;
        coord = (origin.xy + distance_to_plane * ray.xy) / screen.xy + 0.5;
        if (any(lessThan(coord, vec2(0.0))) || any(greaterThan(coord, vec2(1.0)))) {
            return;
        }
    } else {
        float longitude = atan(ray.x, -ray.z);
        float latitude = asin(clamp(ray.y, -1.0, 1.0));
        if (projection == 1 && abs(longitude) > PI * 0.5) {
            return;
        }
        float span = projection == 1 ? PI : 2.0 * PI;
        coord = vec2(longitude / span, latitude / PI) + 0.5;
    }
    vec3 rgb = sample_eye(coord);
    color = vec4(linear_target ? srgb_to_linear(rgb) : rgb, 1.0);
}
