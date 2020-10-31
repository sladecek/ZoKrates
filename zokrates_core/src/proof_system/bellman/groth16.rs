use bellman::groth16::{
    prepare_verifying_key, verify_proof, Parameters, PreparedVerifyingKey, Proof as BellmanProof,
    VerifyingKey,
};
use pairing::{CurveAffine, Engine};
use regex::Regex;

use zokrates_field::Field;

use crate::ir;
use crate::proof_system::bellman::Computation;
use crate::proof_system::bellman::{parse_fr, parse_g1, parse_g2};
use crate::proof_system::solidity::{
    SOLIDITY_G2_ADDITION_LIB, SOLIDITY_PAIRING_LIB, SOLIDITY_PAIRING_LIB_V2,
};
use proof_system::{G1Affine, G2Affine, Proof, ProofSystem, SetupKeypair, SolidityAbi};

const G16_WARNING: &str = "WARNING: You are using the G16 scheme which is subject to malleability. See zokrates.github.io/toolbox/proving_schemes.html#g16-malleability for implications.";

pub struct G16 {}

#[derive(Serialize, Deserialize)]
pub struct ProofPoints {
    a: G1Affine,
    b: G2Affine,
    c: G1Affine,
}

impl ProofPoints {
    pub fn into_bellman<T: Field>(self) -> BellmanProof<T::BellmanEngine> {
        BellmanProof {
            a: serialization::to_g1::<T>(self.a),
            b: serialization::to_g2::<T>(self.b),
            c: serialization::to_g1::<T>(self.c),
        }
    }

    pub fn from_bellman<T: Field>(proof: &BellmanProof<T::BellmanEngine> ) -> Self {
        ProofPoints {
            a: parse_g1::<T>(&proof.a),
            b: parse_g2::<T>(&proof.b),
            c: parse_g1::<T>(&proof.c),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct VerificationKey {
    alpha: G1Affine,
    beta: G2Affine,
    gamma: G2Affine,
    delta: G2Affine,
    gamma_abc: Vec<G1Affine>,
}

impl VerificationKey {
    fn into_bellman<T: Field>(self) -> VerifyingKey<T::BellmanEngine> {
        VerifyingKey {
            alpha_g1: serialization::to_g1::<T>(self.alpha),
            beta_g1: <T::BellmanEngine as Engine>::G1Affine::one(), // not used during verification
            beta_g2: serialization::to_g2::<T>(self.beta),
            gamma_g2: serialization::to_g2::<T>(self.gamma),
            delta_g1: <T::BellmanEngine as Engine>::G1Affine::one(), // not used during verification
            delta_g2: serialization::to_g2::<T>(self.delta),
            ic: self
                .gamma_abc
                .into_iter()
                .map(|g1| serialization::to_g1::<T>(g1))
                .collect(),
        }
    }
}

impl<T: Field> ProofSystem<T> for G16 {
    type VerificationKey = VerificationKey;
    type ProofPoints = ProofPoints;

    fn setup(program: ir::Prog<T>) -> SetupKeypair<VerificationKey> {
        #[cfg(not(target_arch = "wasm32"))]
        std::env::set_var("BELLMAN_VERBOSE", "0");
        println!("{}", G16_WARNING);

        let parameters = Computation::without_witness(program).setup();

        let mut pk: Vec<u8> = Vec::new();

        parameters.write(&mut pk).unwrap();

        let vk = VerificationKey {
            alpha: parse_g1::<T>(&parameters.vk.alpha_g1),
            beta: parse_g2::<T>(&parameters.vk.beta_g2),
            gamma: parse_g2::<T>(&parameters.vk.gamma_g2),
            delta: parse_g2::<T>(&parameters.vk.delta_g2),
            gamma_abc: parameters
                .vk
                .ic
                .iter()
                .map(|g1| parse_g1::<T>(g1))
                .collect(),
        };

        SetupKeypair::new(vk, pk)
    }

    fn generate_proof(
        program: ir::Prog<T>,
        witness: ir::Witness<T>,
        proving_key: Vec<u8>,
    ) -> Proof<ProofPoints> {
        #[cfg(not(target_arch = "wasm32"))]
        std::env::set_var("BELLMAN_VERBOSE", "0");

        println!("{}", G16_WARNING);

        let computation = Computation::with_witness(program, witness);
        let params = Parameters::read(proving_key.as_slice(), true).unwrap();

        let proof = computation.clone().prove(&params);
        let proof_points = ProofPoints::from_bellman::<T>(&proof);

        let inputs = computation
            .public_inputs_values()
            .iter()
            .map(parse_fr::<T>)
            .collect::<Vec<_>>();

        let mut raw: Vec<u8> = Vec::new();
        proof.write(&mut raw).unwrap();

        Proof::<ProofPoints>::new(proof_points, inputs, hex::encode(&raw))
    }

    fn export_solidity_verifier(vk: VerificationKey, abi: SolidityAbi) -> String {
        let (mut template_text, solidity_pairing_lib) = match abi {
            SolidityAbi::V1 => (
                String::from(CONTRACT_TEMPLATE),
                String::from(SOLIDITY_PAIRING_LIB),
            ),
            SolidityAbi::V2 => (
                String::from(CONTRACT_TEMPLATE_V2),
                String::from(SOLIDITY_PAIRING_LIB_V2),
            ),
        };

        let vk_regex = Regex::new(r#"(<%vk_[^i%]*%>)"#).unwrap();
        let vk_gamma_abc_len_regex = Regex::new(r#"(<%vk_gamma_abc_length%>)"#).unwrap();
        let vk_gamma_abc_repeat_regex = Regex::new(r#"(<%vk_gamma_abc_pts%>)"#).unwrap();
        let vk_input_len_regex = Regex::new(r#"(<%vk_input_length%>)"#).unwrap();
        let input_loop = Regex::new(r#"(<%input_loop%>)"#).unwrap();
        let input_argument = Regex::new(r#"(<%input_argument%>)"#).unwrap();

        template_text = vk_regex
            .replace(template_text.as_str(), vk.alpha.to_string().as_str())
            .into_owned();

        template_text = vk_regex
            .replace(template_text.as_str(), vk.beta.to_string().as_str())
            .into_owned();

        template_text = vk_regex
            .replace(template_text.as_str(), vk.gamma.to_string().as_str())
            .into_owned();

        template_text = vk_regex
            .replace(template_text.as_str(), vk.delta.to_string().as_str())
            .into_owned();

        let gamma_abc_count: usize = vk.gamma_abc.len();
        template_text = vk_gamma_abc_len_regex
            .replace(
                template_text.as_str(),
                format!("{}", gamma_abc_count).as_str(),
            )
            .into_owned();

        template_text = vk_input_len_regex
            .replace(
                template_text.as_str(),
                format!("{}", gamma_abc_count - 1).as_str(),
            )
            .into_owned();

        // feed input values only if there are any
        template_text = if gamma_abc_count > 1 {
            input_loop.replace(
                template_text.as_str(),
                r#"
        for(uint i = 0; i < input.length; i++){
            inputValues[i] = input[i];
        }"#,
            )
        } else {
            input_loop.replace(template_text.as_str(), "")
        }
        .to_string();

        // take input values as argument only if there are any
        template_text = if gamma_abc_count > 1 {
            input_argument.replace(
                template_text.as_str(),
                format!(", uint[{}] memory input", gamma_abc_count - 1).as_str(),
            )
        } else {
            input_argument.replace(template_text.as_str(), "")
        }
        .to_string();

        let mut gamma_abc_repeat_text = String::new();
        for (i, g1) in vk.gamma_abc.iter().enumerate() {
            gamma_abc_repeat_text.push_str(
                format!(
                    "vk.gamma_abc[{}] = Pairing.G1Point({});",
                    i,
                    g1.to_string().as_str()
                )
                .as_str(),
            );
            if i < gamma_abc_count - 1 {
                gamma_abc_repeat_text.push_str("\n        ");
            }
        }

        template_text = vk_gamma_abc_repeat_regex
            .replace(template_text.as_str(), gamma_abc_repeat_text.as_str())
            .into_owned();

        let re = Regex::new(r"(?P<v>0[xX][0-9a-fA-F]{64})").unwrap();
        template_text = re.replace_all(&template_text, "uint256($v)").to_string();

        format!(
            "{}{}{}",
            SOLIDITY_G2_ADDITION_LIB, solidity_pairing_lib, template_text
        )
    }

    fn verify(vk: VerificationKey, proof: Proof<ProofPoints>) -> bool {
        let vk: VerifyingKey<T::BellmanEngine> = vk.into_bellman::<T>();

        let pvk: PreparedVerifyingKey<T::BellmanEngine> = prepare_verifying_key(&vk);

        let bellman_proof: BellmanProof<T::BellmanEngine> = proof.proof.into_bellman::<T>();

        let public_inputs: Vec<_> = proof
            .inputs
            .iter()
            .map(|s| {
                T::try_from_str(s.trim_start_matches("0x"), 16)
                    .expect(format!("Invalid {} value: {}", T::name(), s).as_str())
                    .into_bellman()
            })
            .collect::<Vec<_>>();

        verify_proof(&pvk, &bellman_proof, &public_inputs).unwrap()
    }
}

mod serialization {
    use pairing::{from_hex, CurveAffine, Engine};
    use proof_system::{G1Affine, G2Affine};
    use zokrates_field::Field;

    pub fn to_g1<T: Field>(g1: G1Affine) -> <T::BellmanEngine as Engine>::G1Affine {
        <T::BellmanEngine as Engine>::G1Affine::from_xy_checked(
            from_hex(&g1.0).unwrap(),
            from_hex(&g1.1).unwrap(),
        )
        .unwrap()
    }
    pub fn to_g2<T: Field>(g2: G2Affine) -> <T::BellmanEngine as Engine>::G2Affine {
        // apparently the order is reversed
        let x = T::new_fq2(&(g2.0).1, &(g2.0).0);
        let y = T::new_fq2(&(g2.1).1, &(g2.1).0);
        <T::BellmanEngine as Engine>::G2Affine::from_xy_checked(x, y).unwrap()
    }
}

const CONTRACT_TEMPLATE_V2: &str = r#"
contract Verifier {
    using Pairing for *;
    struct VerifyingKey {
        Pairing.G1Point alpha;
        Pairing.G2Point beta;
        Pairing.G2Point gamma;
        Pairing.G2Point delta;
        Pairing.G1Point[] gamma_abc;
    }
    struct Proof {
        Pairing.G1Point a;
        Pairing.G2Point b;
        Pairing.G1Point c;
    }
    function verifyingKey() pure internal returns (VerifyingKey memory vk) {
        vk.alpha = Pairing.G1Point(<%vk_alpha%>);
        vk.beta = Pairing.G2Point(<%vk_beta%>);
        vk.gamma = Pairing.G2Point(<%vk_gamma%>);
        vk.delta = Pairing.G2Point(<%vk_delta%>);
        vk.gamma_abc = new Pairing.G1Point[](<%vk_gamma_abc_length%>);
        <%vk_gamma_abc_pts%>
    }
    function verify(uint[] memory input, Proof memory proof) internal view returns (uint) {
        uint256 snark_scalar_field = 21888242871839275222246405745257275088548364400416034343698204186575808495617;
        VerifyingKey memory vk = verifyingKey();
        require(input.length + 1 == vk.gamma_abc.length);
        // Compute the linear combination vk_x
        Pairing.G1Point memory vk_x = Pairing.G1Point(0, 0);
        for (uint i = 0; i < input.length; i++) {
            require(input[i] < snark_scalar_field);
            vk_x = Pairing.addition(vk_x, Pairing.scalar_mul(vk.gamma_abc[i + 1], input[i]));
        }
        vk_x = Pairing.addition(vk_x, vk.gamma_abc[0]);
        if(!Pairing.pairingProd4(
             proof.a, proof.b,
             Pairing.negate(vk_x), vk.gamma,
             Pairing.negate(proof.c), vk.delta,
             Pairing.negate(vk.alpha), vk.beta)) return 1;
        return 0;
    }
    function verifyTx(
            Proof memory proof<%input_argument%>
        ) public view returns (bool r) {
        uint[] memory inputValues = new uint[](input.length);
        <%input_loop%>
        if (verify(inputValues, proof) == 0) {
            return true;
        } else {
            return false;
        }
    }
}
"#;

const CONTRACT_TEMPLATE: &str = r#"
contract Verifier {
    using Pairing for *;
    struct VerifyingKey {
        Pairing.G1Point alpha;
        Pairing.G2Point beta;
        Pairing.G2Point gamma;
        Pairing.G2Point delta;
        Pairing.G1Point[] gamma_abc;
    }
    struct Proof {
        Pairing.G1Point a;
        Pairing.G2Point b;
        Pairing.G1Point c;
    }
    function verifyingKey() pure internal returns (VerifyingKey memory vk) {
        vk.alpha = Pairing.G1Point(<%vk_alpha%>);
        vk.beta = Pairing.G2Point(<%vk_beta%>);
        vk.gamma = Pairing.G2Point(<%vk_gamma%>);
        vk.delta = Pairing.G2Point(<%vk_delta%>);
        vk.gamma_abc = new Pairing.G1Point[](<%vk_gamma_abc_length%>);
        <%vk_gamma_abc_pts%>
    }
    function verify(uint[] memory input, Proof memory proof) internal view returns (uint) {
        uint256 snark_scalar_field = 21888242871839275222246405745257275088548364400416034343698204186575808495617;
        VerifyingKey memory vk = verifyingKey();
        require(input.length + 1 == vk.gamma_abc.length);
        // Compute the linear combination vk_x
        Pairing.G1Point memory vk_x = Pairing.G1Point(0, 0);
        for (uint i = 0; i < input.length; i++) {
            require(input[i] < snark_scalar_field);
            vk_x = Pairing.addition(vk_x, Pairing.scalar_mul(vk.gamma_abc[i + 1], input[i]));
        }
        vk_x = Pairing.addition(vk_x, vk.gamma_abc[0]);
        if(!Pairing.pairingProd4(
             proof.a, proof.b,
             Pairing.negate(vk_x), vk.gamma,
             Pairing.negate(proof.c), vk.delta,
             Pairing.negate(vk.alpha), vk.beta)) return 1;
        return 0;
    }
    function verifyTx(
            uint[2] memory a,
            uint[2][2] memory b,
            uint[2] memory c<%input_argument%>
        ) public view returns (bool r) {
        Proof memory proof;
        proof.a = Pairing.G1Point(a[0], a[1]);
        proof.b = Pairing.G2Point([b[0][0], b[0][1]], [b[1][0], b[1][1]]);
        proof.c = Pairing.G1Point(c[0], c[1]);
        uint[] memory inputValues = new uint[](<%vk_input_length%>);
        <%input_loop%>
        if (verify(inputValues, proof) == 0) {
            return true;
        } else {
            return false;
        }
    }
}
"#;

#[cfg(test)]
mod tests {
    use crate::flat_absy::FlatVariable;
    use crate::ir::{Function, Interpreter, Prog, Statement};

    use super::*;
    use zokrates_field::Bn128Field;

    #[test]
    fn verify() {
        let program: Prog<Bn128Field> = Prog {
            main: Function {
                id: String::from("main"),
                arguments: vec![FlatVariable::new(0)],
                returns: vec![FlatVariable::public(0)],
                statements: vec![Statement::Constraint(
                    FlatVariable::new(0).into(),
                    FlatVariable::public(0).into(),
                )],
            },
            private: vec![false],
        };

        let keypair = G16::setup(program.clone());

        let interpreter = Interpreter::default();

        let witness = interpreter
            .execute(&program, &vec![Bn128Field::from(42)])
            .unwrap();

        let proof = G16::generate_proof(program, witness, keypair.pk);
        let ans = <G16 as ProofSystem<Bn128Field>>::verify(keypair.vk, proof);

        assert!(ans);
    }
}
