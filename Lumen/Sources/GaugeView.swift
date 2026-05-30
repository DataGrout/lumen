import SwiftUI

struct ArcGauge: View {
    let value: Double
    let maxValue: Double
    let label: String
    let unit: String
    let prefix: String
    let color: Color
    let size: CGFloat

    init(value: Double, max: Double, label: String, unit: String = "",
         prefix: String = "", color: Color = .green, size: CGFloat = 110) {
        self.value = value
        self.maxValue = Swift.max(max, 0.001)
        self.label = label
        self.unit = unit
        self.prefix = prefix
        self.color = color
        self.size = size
    }

    private var fraction: Double {
        min(value / maxValue, 1.0)
    }

    private var displayValue: String {
        if value >= 1_000_000 {
            return String(format: "%.1fM", value / 1_000_000)
        } else if value >= 1_000 {
            return String(format: "%.1fk", value / 1_000)
        } else if value < 10 {
            return String(format: "%.2f", value)
        } else {
            return String(format: "%.0f", value)
        }
    }

    var body: some View {
        ZStack {
            // Background arc
            ArcShape(fraction: 1.0)
                .stroke(Color.white.opacity(0.08), style: StrokeStyle(lineWidth: 7, lineCap: .round))
                .frame(width: size, height: size)

            // Value arc
            ArcShape(fraction: fraction)
                .stroke(color, style: StrokeStyle(lineWidth: 7, lineCap: .round))
                .shadow(color: color.opacity(0.4), radius: 6)
                .frame(width: size, height: size)
                .animation(.easeInOut(duration: 0.5), value: fraction)

            VStack(spacing: 1) {
                Text(prefix + displayValue)
                    .font(.system(size: size * 0.19, weight: .bold, design: .rounded))
                    .foregroundStyle(color)
                    .monospacedDigit()

                if !unit.isEmpty {
                    Text(unit)
                        .font(.system(size: size * 0.09))
                        .foregroundStyle(.white.opacity(0.4))
                        .textCase(.uppercase)
                }

                Text(label)
                    .font(.system(size: size * 0.09))
                    .foregroundStyle(.white.opacity(0.5))
                    .textCase(.uppercase)
                    .tracking(0.5)
            }
        }
    }
}

struct ArcShape: Shape {
    var fraction: Double

    var animatableData: Double {
        get { fraction }
        set { fraction = newValue }
    }

    func path(in rect: CGRect) -> Path {
        let center = CGPoint(x: rect.midX, y: rect.midY)
        let radius = min(rect.width, rect.height) / 2 - 4
        let startAngle = Angle(degrees: 135)
        let endAngle = Angle(degrees: 135 + 270 * fraction)

        var path = Path()
        path.addArc(center: center, radius: radius,
                    startAngle: startAngle, endAngle: endAngle,
                    clockwise: false)
        return path
    }
}
