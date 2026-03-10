pub mod error;
pub mod gradient;
pub mod optimizer;
pub mod similarity;
pub mod statistics;
pub mod symbolic;
pub mod tensor_ops;

pub use error::{CalculusError, Result};
pub use gradient::{apply_gradient, clip_gradient, gradient_alignment, gradient_norm, ste_gradient};
pub use optimizer::{analyze_quantization, find_threshold_for_sparsity, weight_balance, weight_entropy, QuantizationStats};
pub use similarity::{cosine_similarity, jaccard_index, knn_cosine, sign_agreement};
pub use statistics::{batch_mean, batch_stddev, batch_variance, iqr_outliers, percentile, Ema, RunningStats};
pub use symbolic::{Expr, PolicyRule};
pub use tensor_ops::{ternary_add_saturate, ternary_dot, ternary_hamming, ternary_l1_norm, ternary_sparsity};
